package dev.r1os.host

import android.os.Handler
import android.os.Looper

/**
 * An agent's pointer (T-137; Max, 2026-10-12: "an overlay imaginary pointer/cursor showing their moves, with smooth
 * transitions between events"): where an agent is about to act in the R1's browser ([browser]) or the app window
 * ([app]). The host draws it over whatever shows that surface ([Mark]: the chat's preview, the full screen), never into
 * the page or the app, so screenshots and what the agent reads never have it.
 *
 * An act waits for the pointer to land, so what you see is cause then effect, at most [MAX_GLIDE_MS]. With nobody
 * watching (no preview on screen, the display dimmed or off) it doesn't wait, and nothing's drawn.
 */
class AgentPointer {
    /** Something drawing this surface's pointer: a view over a preview or a full screen. */
    interface Mark {
        /** On screen and seen (not dimmed). */
        val watched: Boolean

        /** Glides to [x],[y] (the surface's own pixels) from wherever it is; how long that takes, in ms. */
        fun glide(x: Float, y: Float, fromX: Float, fromY: Float): Long

        /** It landed on what it taps. */
        fun press()

        /** Along a drag to [x],[y] over [ms] (a swipe, a scroll). */
        fun drag(x: Float, y: Float, ms: Long)

        /** A small pulse where it rests (a key). */
        fun pulse()
    }

    private val marks = LinkedHashSet<Mark>()
    private val main = Handler(Looper.getMainLooper())
    /** Where it last pointed, in the surface's own pixels: a mark that comes on screen starts there. */
    private var x = Float.NaN
    private var y = Float.NaN

    fun add(m: Mark) = marks.add(m)

    fun remove(m: Mark) = marks.remove(m)

    private fun seen() = marks.filter { it.watched }

    /** Whether anyone would see it. */
    val watched: Boolean get() = marks.any { it.watched }

    /** Points at [x],[y], then [act] as it lands ([press]: a tap); at once with nobody watching. */
    fun aim(x: Float, y: Float, press: Boolean = true, act: () -> Unit) {
        val seen = seen()
        val fromX = this.x
        val fromY = this.y
        this.x = x
        this.y = y
        if (seen.isEmpty()) {
            act()
            return
        }
        val ms = seen.maxOf { it.glide(x, y, fromX, fromY) }.coerceAtMost(MAX_GLIDE_MS)
        main.postDelayed({
            if (press) seen.forEach { it.press() }
            act()
        }, ms)
    }

    /** A drag from [x0],[y0] to [x1],[y1] over [ms] (a swipe): it goes to the start, then along as [act] does it. */
    fun drag(x0: Float, y0: Float, x1: Float, y1: Float, ms: Long, act: () -> Unit) {
        aim(x0, y0, press = false) {
            act()
            seen().forEach { it.drag(x1, y1, ms) }
            x = x1
            y = y1
        }
    }

    /** A key: a pulse where it rests. */
    fun pulse() = seen().forEach { it.pulse() }

    companion object {
        /** The R1's browser's (AgentBrowser) and the app window's (AgentApps). */
        val browser = AgentPointer()
        val app = AgentPointer()
        /** The longest an act waits for the pointer to land. */
        const val MAX_GLIDE_MS = 450L
    }
}
