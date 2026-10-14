package dev.r1os.host.ui

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.view.View
import android.view.ViewGroup

/**
 * Whether Android has dimmed the display (T-036). It dims a while before the screen timeout with no input and tells
 * no app, so the host keeps the same clock: any key or touch (the wheel and the button too, though the shell swallows
 * them) and the display coming on start it again; [MAX_DIM_MS] (or [DIM_RATIO] of a short timeout) before the timeout
 * it's dimmed, and characters hold still until the next input. Not while a view keeps the screen on: Android doesn't
 * dim then. One timer, armed from the last input, never a tick.
 *
 * And the glance screen at rest (T-157; Max, Q67: "dim some features to reduce battery load after some time but still
 * show a dispay on there so it can act as an AOD"): while [rest] is on, the shell keeps the display on and this steps
 * it down from the last input, Android's dim aside: still after [STILL_MS] ([dimmed]: nothing moves), low after
 * [LOW_MS] ([low]: the shell turns the backlight down and the accelerometer off). Input brings it back.
 */
object Dim {
    /** Android's defaults on the R1 (`dumpsys power`: mMaximumScreenDimDurationConfig, mMaximumScreenDimRatioConfig). */
    private const val MAX_DIM_MS = 7_000L
    private const val DIM_RATIO = 0.2
    /**
     * At rest (T-157): how long untouched before nothing moves, and before the backlight goes low. Together: the
     * characters' calm life costs a few mA on one clock (it was ~15 at 19 frames a second), and Max likes them alive.
     */
    const val STILL_MS = 120_000L
    const val LOW_MS = 120_000L

    var dimmed = false
        private set
    /** At rest, the lowest step: the backlight low, the accelerometer off (the shell's). */
    var low = false
        private set
    /** Whether the last input found it low: that touch only wakes it (the shell swallows it). */
    var wasLow = false
        private set
    /** The glance screen as an always-on display (the shell's word). */
    var resting = false
        private set
    /** The harness's step (`/harness/rest`: 0 awake, 1 still, 2 low) over the timer, at rest; null: the timer's. */
    private var forced: Int? = null
    private var last = 0L
    private var armed = false
    private var displayOn = false
    private var timeoutMs = 60_000L
    private var context: Context? = null
    private val main = Handler(Looper.getMainLooper())
    private val watchers = LinkedHashSet<() -> Unit>()

    /** Where views that keep the screen on would be: the shell, and its dock when it's lifted over another app. */
    var roots: List<View> = emptyList()

    fun watch(f: () -> Unit) {
        watchers += f
    }

    fun unwatch(f: () -> Unit) {
        watchers -= f
    }

    /** The display came on: the timeout may have changed while it was off. */
    fun on(c: Context) {
        context = c.applicationContext
        displayOn = true
        readTimeout()
        poke()
    }

    /** The display went off: nothing to time until it's on again. */
    fun off() {
        displayOn = false
        main.removeCallbacks(check)
        armed = false
    }

    /** Input: Android's timer starts again, and so does this. Only a timestamp when it's already armed (wheel ticks). */
    fun poke() {
        last = SystemClock.uptimeMillis()
        wasLow = low
        if (forced != null) return
        if (dimmed || low) set(false, false)
        if (!armed && displayOn) arm(if (resting) STILL_MS else dimAfter())
    }

    /** The glance screen is the always-on display now (docked: T-157), or isn't: the steps start from the last input. */
    fun rest(on: Boolean) {
        if (on == resting) return
        resting = on
        if (!on) set(false, false)
        again()
    }

    /** The harness holds a step (0 awake, 1 still, 2 low), or (null) hands it back to the timer. */
    fun force(step: Int?) {
        forced = step
        again()
    }

    /** The timer from the last input again, now. */
    private fun again() {
        main.removeCallbacks(check)
        armed = false
        if (displayOn) arm(0)
    }

    private fun readTimeout() {
        val c = context ?: return
        timeoutMs = runCatching { Settings.System.getInt(c.contentResolver, Settings.System.SCREEN_OFF_TIMEOUT).toLong() }.getOrDefault(timeoutMs)
    }

    private fun dimAfter(): Long = timeoutMs - minOf(MAX_DIM_MS, (timeoutMs * DIM_RATIO).toLong())

    private fun arm(ms: Long) {
        armed = true
        main.postDelayed(check, ms.coerceAtLeast(0))
    }

    private val check = Runnable {
        armed = false
        val now = SystemClock.uptimeMillis()
        if (resting) {
            val f = forced
            val still = last + STILL_MS - now
            val lowIn = last + LOW_MS - now
            when {
                f != null -> set(f >= 1, f >= 2)
                still > 0 -> { set(false, false); arm(still) }
                lowIn > 0 -> { set(true, false); arm(lowIn) }
                else -> set(true, true)
            }
            return@Runnable
        }
        readTimeout()
        val left = last + dimAfter() - now
        when {
            left > 0 -> arm(left)
            // Android doesn't dim while a view keeps the screen on (an alarm ringing); the next input arms this again.
            roots.any(::keepsOn) -> {}
            else -> set(true, false)
        }
    }

    private fun keepsOn(v: View): Boolean =
        v.isShown && (v.keepScreenOn || (v is ViewGroup && (0 until v.childCount).any { keepsOn(v.getChildAt(it)) }))

    private fun set(d: Boolean, l: Boolean) {
        if (d == dimmed && l == low) return
        dimmed = d
        low = l
        watchers.toList().forEach { it() }
    }
}
