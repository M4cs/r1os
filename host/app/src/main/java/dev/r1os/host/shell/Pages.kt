package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.SvgPath
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import org.json.JSONObject

/**
 * A page below home (ADR-0014): its title as header text, its content below. One wheel tick past the
 * top of the content brings Back over the header; while it shows, a click goes back and a tick down
 * hides it. Double tap goes back too (the shell's).
 */
abstract class PageScreen(shell: Shell, title: String, accent: Int = Theme.accent) : Screen(shell) {
    protected val header = BackHeader(context, title, accent)
    protected val body = FrameLayout(context)
    override val view = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundColor(Theme.BG)
        addView(header, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            setMargins(context.dpi(14f - BackHeader.INSET), context.dpi(10f), context.dpi(14f), context.dpi(2f))
        })
        addView(body, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
    }

    init {
        header.onBackTouched(light = { overscroll() }, out = { header.hide(); onBackLit(false) }) { shell.back() }
    }

    override val backHeader get() = header

    /** The page's own input, once Back isn't showing. True when consumed. */
    open fun onInput(ev: Ev): Boolean = false

    final override fun onEvent(ev: Ev): Boolean {
        if (header.shown) {
            when {
                ev.type == "wheel" -> { if (ev.int("delta") > 0) { header.hide(); onBackLit(false) }; return true }
                ev.isTap -> { shell.back(); return true }
            }
        }
        return onInput(ev)
    }

    /** The wheel went past the top: show Back. */
    fun overscroll() {
        header.show()
        onBackLit(true)
    }

    /**
     * The title lit up as Back (`lit`), or went out. Only one thing is highlighted at a time: the
     * page's own highlight steps aside while Back is lit, and comes back on its first row.
     */
    open fun onBackLit(lit: Boolean) {}

    override fun onHide() = header.hide()
}

/** A page that's one wheel list: the wheel moves the focus, a click activates it. */
abstract class ListPage(shell: Shell, title: String, accent: Int = Theme.accent) : PageScreen(shell, title, accent) {
    protected val list = FocusList(context).apply {
        this.accent = accent
        setPadding(context.dpi(10f), context.dpi(4f), context.dpi(10f), context.dpi(20f))
    }

    init {
        body.addView(list, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }

    private var later: (() -> Unit)? = null
    private val runLater = Runnable { later?.also { later = null }?.invoke() }

    /**
     * Renders with what r1d sent, once for everything that lands together: a page's loads arrive as it opens, and a
     * rebuild each (tens of ms for a long list on the R1) took the slide-in's frames (T-041). It runs at the next turn of
     * the main loop, after the answers already waiting there.
     */
    protected fun soon(render: () -> Unit) {
        val first = later == null
        later = render
        if (first) list.post(runLater)
    }

    /**
     * GETs [path] for this page (T-106): [use] gets what r1d answered last time at once, so the page's first build has
     * it, and then the new answer, rendering ([soon]) only if it's different. A rebuild of a long list is what makes a
     * page's slide-in stutter on the R1, and most answers haven't changed since the last visit.
     */
    protected fun fetch(path: String, render: () -> Unit, onError: (String) -> Unit = {}, use: (JSONObject) -> Unit) {
        Seen.get(path)?.let { runCatching { use(JSONObject(it)) } }
        R1d.get(path, onError = onError) { r ->
            if (Seen.put(path, r.toString())) {
                use(r)
                soon(render)
            }
        }
    }

    /** Input the list doesn't take: held-button words, mostly. */
    open fun extra(ev: Ev): Boolean = false

    override fun onBackLit(lit: Boolean) = backLit(list, lit)

    override fun onInput(ev: Ev): Boolean {
        if (extra(ev)) return true
        return listInput(list, ev) { overscroll() }
    }

    override fun focusIndex() = list.focus
    override fun restoreFocus(i: Int) {
        if (i in 0 until list.size) list.setFocus(i, animate = false)
    }
}

/** r1d's last answers to the pages' GETs, by path ([ListPage.fetch]): the newest 48. */
object Seen {
    private val last = object : LinkedHashMap<String, String>(64, 0.75f, true) {
        override fun removeEldestEntry(e: MutableMap.MutableEntry<String, String>) = size > 48
    }

    fun get(path: String): String? = last[path]

    /** Keeps [body] as [path]'s answer: true if it's new or different. */
    fun put(path: String, body: String): Boolean = last.put(path, body) != body
}

/** Back is lit: the list's highlight steps aside. Back went out: it returns to the first row. */
fun backLit(list: FocusList, lit: Boolean) {
    if (lit) list.blur() else if (list.size > 0) list.setFocus(0)
}

/** The usual list input: wheel = focus (past the top = `onTop`), click = activate. */
fun listInput(list: FocusList, ev: Ev, onTop: () -> Unit): Boolean {
    if (ev.type == "wheel") {
        val d = if (ev.int("delta") > 0) 1 else -1
        if (!list.move(d) && d < 0) onTop()
        return true
    }
    if (ev.isTap) return list.activate()
    return false
}

/**
 * A step of setup that onboarding and Settings share (Wi-Fi, making an agent, connecting a brain):
 * its own view and input, hosted by a [PanelPage] or by onboarding.
 */
abstract class Panel(val shell: Shell) {
    val context: Context get() = shell.context
    abstract val view: View
    /** The wheel went past the top (a page shows Back; onboarding ignores it). */
    var onTop: () -> Unit = {}
    open fun onInput(ev: Ev): Boolean = false
    open fun onAnyEvent(ev: Ev) {}
    open fun onShow() {}
    open fun onHide() {}
    open fun onDestroy() {}
    /** A double tap the panel uses itself (backing out of a sub-step). */
    open fun onBack(): Boolean = false

    /** Its page's title lit up as Back, or went out ([PageScreen.onBackLit]). */
    open fun onBackLit(lit: Boolean) {}
}

/** A [Panel] as a page of its own (Settings › Wi-Fi, an agent's look, New agent). */
class PanelPage(shell: Shell, title: String, accent: Int, val panel: Panel, override val agentId: String? = null) : PageScreen(shell, title, accent) {
    init {
        body.addView(panel.view, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        panel.onTop = { overscroll() }
    }

    override fun onInput(ev: Ev): Boolean {
        if (ev.isDoubleTap && panel.onBack()) return true
        return panel.onInput(ev)
    }

    override fun onAnyEvent(ev: Ev) = panel.onAnyEvent(ev)
    override fun onBackLit(lit: Boolean) = panel.onBackLit(lit)
    override fun onShow() = panel.onShow()
    override fun onHide() { super.onHide(); panel.onHide() }
    override fun onDestroy() = panel.onDestroy()
}

/** A row tapped twice to confirm (delete, remove): the first tap arms it for 3 s. */
class Confirm(private val onChange: () -> Unit) {
    var armed: String? = null
        private set
    private val disarm = Runnable { armed = null; onChange() }

    fun tap(key: String, run: () -> Unit) {
        R1d.main.removeCallbacks(disarm)
        if (armed == key) {
            armed = null
            run()
        } else {
            armed = key
            R1d.main.postDelayed(disarm, 3000)
        }
        onChange()
    }

    fun cancel() = R1d.main.removeCallbacks(disarm)
}

/** "now", "5m", "3h", "2d" (an agent's chats); `long`: "just now", "5 min ago" (an app's history). */
fun ago(ms: Long, long: Boolean = false): String {
    val s = ((System.currentTimeMillis() - ms) / 1000).coerceAtLeast(0)
    return when {
        s < 60 -> if (long) "just now" else "now"
        s < 3600 -> if (long) "${s / 60} min ago" else "${s / 60}m"
        s < 86400 -> if (long) "${s / 3600} h ago" else "${s / 3600}h"
        else -> if (long) "${s / 86400} d ago" else "${s / 86400}d"
    }
}

/** A filled card (`.row`, `.brain` in the web shell): a column of views on #121418. */
fun Context.card(vararg views: View, ok: Boolean = false, accent: Int? = null): LinearLayout = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    setPadding(dpi(14f), dpi(12f), dpi(14f), dpi(12f))
    background = roundRect(Theme.PANEL, dp(16f), if (ok || accent != null) dpi(2f) else 0, if (ok) Theme.OK else accent ?: 0)
    views.forEach { addView(it) }
}

/**
 * A QR code, drawn natively from r1d's `/api/qr` SVG (one white rect, one black path: the qrcode
 * crate's renderer) on a white rounded tile.
 */
@SuppressLint("ViewConstructor")
class QrView(context: Context, text: String) : View(context) {
    private var path: Path? = null
    private var size = 0f
    private val dark = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.BLACK }
    private val light = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE }
    private val scaled = Path()

    init {
        R1d.bytes("/api/qr?text=${R1d.enc(text)}") { bytes ->
            val svg = bytes?.let { String(it) } ?: return@bytes
            size = Regex("""viewBox="0 0 (\d+) (\d+)"""").find(svg)?.groupValues?.get(1)?.toFloatOrNull() ?: return@bytes
            path = Regex("""<path[^>]*\sd="([^"]+)"""").find(svg)?.groupValues?.get(1)?.let { SvgPath.parse(it) }
            invalidate()
        }
    }

    override fun onDraw(c: Canvas) {
        val pad = context.dp(6f)
        c.drawRoundRect(0f, 0f, width.toFloat(), height.toFloat(), context.dp(14f), context.dp(14f), light)
        val p = path ?: return
        val s = (minOf(width, height) - 2 * pad) / size
        p.transform(Matrix().apply { setScale(s, s); postTranslate(pad, pad) }, scaled)
        c.drawPath(scaled, dark)
    }
}
