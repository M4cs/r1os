package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.view.View
import android.webkit.JavascriptInterface
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject

/** Events that are input: they go only to the screen on top (and, during onboarding, only there). */
val INPUT_EVENTS = setOf("wheel", "wheel_suppressed", "button", "gesture", "voice", "voice_level", "open")

/**
 * A WebView for a mini-app's page (ADR-0017): created with its screen, and kept a while after it closes
 * ([KeptPages]). The page gets r1d's events from the shell (`window.r1Event`) instead of its own socket,
 * and calls back through `R1Native`.
 */
@SuppressLint("SetJavaScriptEnabled", "ViewConstructor")
class R1Page(screen: Screen, private val url: String, bridge: Any, load: Boolean = true) : FrameLayout(screen.context) {
    val web = WebView(screen.context)
    private var loaded = false
    private var failed = false
    private val splash = screen.context.text(15f, 500, Theme.MUTED).apply {
        text = "Opening…"
        gravity = android.view.Gravity.CENTER
        setBackgroundColor(Theme.BG)
    }

    init {
        setBackgroundColor(Theme.BG)
        web.apply {
            setBackgroundColor(Theme.BG)
            isVerticalScrollBarEnabled = false
            isHorizontalScrollBarEnabled = false
            overScrollMode = View.OVER_SCROLL_NEVER
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.mediaPlaybackRequiresUserGesture = false
            addJavascriptInterface(bridge, "R1Native")
            webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
                    // Only r1d's own pages: anything else stays put.
                    !request.url.toString().startsWith(R1d.BASE)

                override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
                    failed = false
                }

                override fun onPageFinished(view: WebView, url: String) {
                    if (failed) return
                    loaded = true
                    splash.animate().alpha(0f).setDuration(120).withEndAction { splash.visibility = View.GONE }.start()
                }

                override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                    if (!request.isForMainFrame) return
                    failed = true
                    splash.text = "Waiting for r1d…"
                    splash.alpha = 1f
                    splash.visibility = View.VISIBLE
                    removeCallbacks(retry)
                    postDelayed(retry, 1000)
                }
            }
        }
        addView(web, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        addView(splash, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        if (load) web.loadUrl(url)
    }

    private val retry = Runnable { reload() }

    fun reload() {
        loaded = false
        web.loadUrl(url)
    }

    /** The page's script is up (it called `R1Native.ready()`): it can take events now. */
    fun ready() {
        loaded = true
    }

    /** Hands the page one event (`window.r1Event`), if it's listening. */
    fun send(ev: Ev) = js("window.r1Event&&window.r1Event(${ev.json})")

    fun js(code: String) {
        if (loaded) web.evaluateJavascript(code, null)
    }

    fun resume() = web.onResume()
    fun pause() = web.onPause()

    /** It loaded and is running (worth keeping when its screen closes). */
    val healthy get() = loaded && !failed

    fun destroy() {
        removeCallbacks(retry)
        web.stopLoading()
        web.removeJavascriptInterface("R1Native")
        removeView(web)
        web.destroy()
    }
}

/**
 * Mini-app pages kept running after their screen closes (ADR-0029): reopening one of the last few apps
 * shows it at once, as you left it, instead of loading it again.
 *
 * - **A few, for a while:** the [MAX] most recent, each for up to [MAX_AGE_MS]; older ones are destroyed.
 * - **Never stale:** a page whose app changed while it was kept (its data, its live values, a new
 *   version) loads again instead. So does every page after r1d restarts (their tokens are void).
 * - **Quiet:** a kept page is paused, and while no web page is on screen the pages' timers stop too
 *   (`pauseTimers`, for every WebView): nothing ticks in the background.
 */
object KeptPages {
    private const val MAX = 3
    private const val MAX_AGE_MS = 30 * 60_000L

    private class Kept(val page: R1Page, val bridge: PageBridge, val at: Long) {
        var stale = false
    }

    private val kept = LinkedHashMap<String, Kept>()
    /** Web screens on screen now (an app, a link): timers run only while there's one. */
    private val showing = HashSet<Screen>()

    /** The kept page for `app` (taken out), if it's still good. */
    fun take(app: String): Pair<R1Page, PageBridge>? {
        val k = kept.remove(app) ?: return null
        if (k.stale || android.os.SystemClock.uptimeMillis() - k.at > MAX_AGE_MS) {
            k.page.destroy()
            return null
        }
        return k.page to k.bridge
    }

    /** `app`'s screen closed: its page is kept (paused), and the oldest beyond [MAX] goes. */
    fun keep(app: String, page: R1Page, bridge: PageBridge) {
        (page.parent as? android.view.ViewGroup)?.removeView(page)
        page.pause()
        bridge.owner = null
        kept.remove(app)?.page?.destroy()
        kept[app] = Kept(page, bridge, android.os.SystemClock.uptimeMillis())
        while (kept.size > MAX) kept.remove(kept.keys.first())?.page?.destroy()
    }

    /** What changed while pages were kept: theirs load again next time. */
    fun onEvent(ev: Ev) {
        val app = ev.str("app") ?: return
        when (ev.type) {
            "app_installed", "app_removed" -> kept.remove(app)?.page?.destroy()
            "app_changed", "app_services" -> kept[app]?.stale = true
        }
    }

    /** r1d restarted: every kept page's token is void. */
    fun clear() {
        kept.values.forEach { it.page.destroy() }
        kept.clear()
    }

    /**
     * A web screen (its WebView `web`) went on screen or off it: the pages' timers run only while one is
     * on. pauseTimers/resumeTimers are process-wide, so any WebView switches them all.
     */
    fun showing(screen: Screen, on: Boolean, web: WebView) {
        if (on) showing += screen else showing -= screen
        if (showing.isNotEmpty()) web.resumeTimers() else web.pauseTimers()
    }
}

/**
 * The page's way back to the shell (`R1Native`). It outlives a screen: a kept page keeps its bridge, and
 * the screen that shows it next becomes its [owner].
 */
class PageBridge {
    @Volatile var owner: AppScreen? = null
    /** The page's own token (ADR-0026): it reaches only its app's runtime endpoints. */
    @Volatile var token = ""

    @JavascriptInterface
    fun ready() = R1d.main.post { owner?.pageReady() }

    @JavascriptInterface
    fun token(): String = token

    /** The wheel turned up past the app's first control. */
    @JavascriptInterface
    fun overscroll() = R1d.main.post { owner?.overscrolled() }

    /** A component in the app took the wheel (a value wheel, a game; ADR-0038), or gave it back. */
    @JavascriptInterface
    fun capture(on: Boolean) = R1d.main.post { owner?.captured = on }
}

/**
 * An installed mini-app (ADR-0017 decision 4): its own WebView with `app.html`, under a native header.
 * The shell forwards the wheel and clicks, and the app's events (`app_*`: its changes and its live
 * values). Double tap goes back and hold talks (the words go to your agent, told which app was on
 * screen). One tick above the app's first control brings Back (ADR-0014): the page reports it with
 * `R1Native.overscroll()`. While a component in it has the wheel (a value wheel, a game; ADR-0038,
 * `R1Native.capture`), the double click and the button's edges go to the page too, and the double click
 * doesn't go back: it puts the value back, or pauses the game.
 *
 * While it's on screen r1d runs its web data (`open`/`close`, docs/specs/apps.md §6): covered, closed or
 * asleep, nothing fetches (unless it runs in the background, `schedule`). Permissions it declares that you
 * haven't decided on are asked first, on the consent card over it (ADR-0023). Closed, its page is kept a
 * while ([KeptPages]), so reopening it is instant.
 */
class AppScreen(shell: Shell, val appId: String) : Screen(shell) {
    private val header = BackHeader(context, "")
    /** The shell's scale over the system's (ADR-0021): the page zooms to match. */
    private val zoom = context.resources.displayMetrics.densityDpi.toFloat() / android.content.res.Resources.getSystem().displayMetrics.densityDpi
    /** Its page from the last time, still running ([KeptPages]); else a new one. */
    private val reused = KeptPages.take(appId)
    private val bridge = reused?.second ?: PageBridge()
    private val page = reused?.first ?: R1Page(this, "${R1d.BASE}/app.html?zoom=${"%.3f".format(java.util.Locale.ROOT, zoom)}&mode=${if (Theme.light) "light" else "dark"}&font=${Theme.fontName}#${R1d.enc(appId)}", bridge, load = false)
    private val column = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundColor(Theme.BG)
        addView(header, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(14f - BackHeader.INSET), context.dpi(10f), context.dpi(14f), context.dpi(2f))
        })
        addView(page, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
    }
    override val view = FrameLayout(context).apply {
        addView(column, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }
    /** Asking about its permissions, over it. */
    private var consent: ConsentCard? = null
    private var shown = false
    /** A component in the page has the wheel (ADR-0038). */
    var captured = false

    override val backHeader get() = header

    init {
        header.onBackTouched(
            light = { header.show(); page.js("window.r1Focus&&window.r1Focus(false)") },
            out = { header.hide(); page.js("window.r1Focus&&window.r1Focus(true)") },
        ) { shell.back() }
        bridge.owner = this
        if (reused == null) session()
        R1d.get("/api/apps") { r ->
            val apps = r.optJSONArray("apps") ?: return@get
            for (i in 0 until apps.length()) {
                val a = apps.optJSONObject(i) ?: continue
                if (a.optString("id") == appId) {
                    header.setTitle(a.optString("name"))
                    ask(a)
                }
            }
        }
    }

    /** A token for the page, then the page (again, after r1d restarted with new tokens). */
    private fun session() {
        R1d.post("/api/apps/${R1d.enc(appId)}/session", onError = { page.reload() }) { r ->
            bridge.token = r.optString("token")
            page.reload()
        }
    }

    /** Permissions it declares that you haven't decided on: the consent card asks. */
    private fun ask(app: JSONObject) {
        R1d.get("/api/apps/${R1d.enc(appId)}/permissions") { r ->
            val all = r.optJSONArray("permissions") ?: return@get
            val asks = (0 until all.length()).mapNotNull { all.optJSONObject(it) }.filter { it.optString("state") == "ask" }
            if (asks.isEmpty() || consent != null) return@get
            val accent = Theme.COLORS[app.optString("accent")] ?: Theme.accent
            val card = ConsentCard(context, appId, app.optString("name"), app.optString("icon", "app-window"), accent, asks) {
                view.removeView(consent)
                consent = null
            }
            consent = card
            view.addView(card, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        }
    }

    override fun onEvent(ev: Ev): Boolean {
        consent?.let { if (it.onEvent(ev)) return true }
        if (header.shown) {
            when {
                ev.type == "wheel" -> {
                    if (ev.int("delta") > 0) {
                        header.hide()
                        page.js("window.r1Focus&&window.r1Focus(true)")
                    }
                    return true
                }
                ev.isTap -> { shell.back(); return true }
            }
        }
        if (ev.type == "wheel" || ev.isTap) {
            page.send(ev)
            return true
        }
        // A component has the wheel: the double click and the button are its too (a game's controls).
        if (captured && (ev.isDoubleTap || ev.type == "button")) {
            page.send(ev)
            return true
        }
        return false
    }

    override fun onAnyEvent(ev: Ev) {
        if (ev.type.startsWith("app_") && ev.str("app") == appId) page.send(ev)
    }

    override fun onShow() {
        shown = true
        page.resume()
        KeptPages.showing(this, true, page.web)
        R1d.post("/api/apps/${R1d.enc(appId)}/open")
    }

    override fun onHide() {
        shown = false
        // Whatever has the wheel gives it back: a value wheel keeps its value, a game pauses.
        if (captured) {
            page.js("window.r1Release&&window.r1Release()")
            captured = false
        }
        page.pause()
        KeptPages.showing(this, false, page.web)
        header.hide()
        R1d.post("/api/apps/${R1d.enc(appId)}/close")
    }

    override fun onReconnect() {
        session()
        if (shown) R1d.post("/api/apps/${R1d.enc(appId)}/open")
    }

    override fun onDestroy() {
        if (shown) R1d.post("/api/apps/${R1d.enc(appId)}/close")
        KeptPages.showing(this, false, page.web)
        // Kept a while, so reopening it is instant; a page that never loaded isn't worth it.
        if (page.healthy) KeptPages.keep(appId, page, bridge) else page.destroy()
        if (bridge.owner === this) bridge.owner = null
    }

    /** The page's script is up. */
    fun pageReady() = page.ready()

    /** The wheel turned up past the app's first control: Back lights. */
    fun overscrolled() {
        if (shell.top() !== this) return
        header.show()
        page.js("window.r1Focus&&window.r1Focus(false)")
    }
}
