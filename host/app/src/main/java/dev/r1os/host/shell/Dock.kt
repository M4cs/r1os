package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.app.ActivityOptions
import android.content.Intent
import android.graphics.PixelFormat
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import dev.r1os.host.OverActivity
import dev.r1os.host.net.R1d

/**
 * What the shell shows over everything: an app's reminder ([NoticeBanner]), an agent's reply
 * ([AgentReplyCard]), push-to-talk ([VoiceOverlay]) and the volume ([VolumeOverlay]). They share one
 * layer that can leave the shell (ADR-0027).
 *
 * While another Android app is in front, the layer moves over that app, so talking, the volume and
 * reminders work there too. It goes into one of two places:
 *
 * - **An overlay window** (an app-op that r1d grants the host). It's quick, and the app underneath
 *   keeps running as if nothing were there.
 * - **[OverActivity]**, a see-through activity on top, where Android hides overlay windows. Its own
 *   settings and Files ask it to (`HIDE_NON_SYSTEM_OVERLAY_WINDOWS`), so r1d checks the screen
 *   (`GET /api/android/front`) as the layer goes up.
 *
 * The layer comes back to the shell once it has nothing to show, or once r1os is in front again.
 *
 * It covers the screen while something full-screen shows (talking, the volume). Otherwise it covers
 * only the card at the top or bottom. Android drops touches that pass through another app's window, so
 * the rest of the app you're in stays touchable, and touching the card does what it does in the shell.
 */
@SuppressLint("ViewConstructor")
class Dock(private val shell: ViewGroup) : FrameLayout(shell.context) {
    private enum class At { SHELL, WINDOW, ACTIVITY }

    private val wm = context.getSystemService(WindowManager::class.java)
    private var at = At.SHELL
    private var activity: OverActivity? = null
    /** Over another app (in a window or [OverActivity]); else in the shell. */
    val over get() = at != At.SHELL
    /** The shape: the whole screen, or a card's edge (`Gravity.TOP`/`BOTTOM`). */
    private var edge = FULL
    /** Where it sits among the shell's views, to go back to. */
    private var slot = -1

    init {
        current = this
    }

    /** Android lets the host draw over other apps, and start its activity from behind them (r1d grants it). */
    fun allowed(): Boolean = Settings.canDrawOverlays(context)

    /**
     * Over the app in front, covering the screen (you're about to talk, or change the volume). False if
     * Android doesn't allow it: the layer stays in the (hidden) shell.
     */
    fun lift(): Boolean {
        if (over) {
            shape(FULL)
            return true
        }
        if (!allowed()) return false
        slot = shell.indexOfChild(this)
        shell.removeView(this)
        edge = FULL
        if (addWindow()) {
            at = At.WINDOW
            // Android hides overlay windows over some apps: there, the activity instead.
            R1d.get("/api/android/front") { r -> if (at == At.WINDOW && !r.optBoolean("overlay", true)) intoActivity() }
        } else {
            intoActivity()
        }
        return true
    }

    /** Back into the shell (r1os is in front again, or there's nothing left to show). */
    fun land() {
        when (at) {
            At.SHELL -> return
            At.WINDOW -> removeWindow()
            At.ACTIVITY -> activity?.finishNow()
        }
        at = At.SHELL
        activity = null
        (parent as? ViewGroup)?.removeView(this)
        shell.addView(this, slot.coerceIn(0, shell.childCount), ViewGroup.LayoutParams(MATCH, MATCH))
    }

    private fun intoActivity() {
        removeWindow()
        at = At.ACTIVITY
        try {
            context.startActivity(Intent(context, OverActivity::class.java), ActivityOptions.makeCustomAnimation(context, 0, 0).toBundle())
        } catch (e: RuntimeException) {
            Log.w(TAG, "over apps: no activity", e)
            land()
        }
    }

    /** [OverActivity] is up: the layer moves in, unless it's already back in the shell. */
    fun attach(a: OverActivity) {
        if (at != At.ACTIVITY || activity != null) {
            a.finishNow()
            return
        }
        activity = a
        (parent as? ViewGroup)?.removeView(this)
        a.setContentView(this, ViewGroup.LayoutParams(MATCH, MATCH))
        a.shape(edge == FULL, edge)
    }

    /** [OverActivity] went (finished, or Android stopped it): the layer goes back to the shell. */
    fun detached(a: OverActivity) {
        if (activity !== a) return
        activity = null
        if (at == At.ACTIVITY) land()
    }

    /** A view showed or went: the layer takes the shape of what's left, or goes. */
    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        super.onLayout(changed, l, t, r, b)
        if (over) post(reshape)
    }

    private val reshape = Runnable {
        if (!over) return@Runnable
        val shown = (0 until childCount).map { getChildAt(it) }.filter { it.visibility != View.GONE }
        if (shown.isEmpty()) {
            land()
            return@Runnable
        }
        val edges = shown.map { v ->
            val lp = v.layoutParams as LayoutParams
            if (lp.height == MATCH) FULL else lp.gravity and Gravity.VERTICAL_GRAVITY_MASK
        }.toSet()
        shape(edges.singleOrNull()?.takeIf { it == Gravity.TOP || it == Gravity.BOTTOM } ?: FULL)
    }

    private fun shape(e: Int) {
        if (e == edge) return
        edge = e
        when (at) {
            At.WINDOW -> runCatching { wm.updateViewLayout(this, params()) }.onFailure { Log.w(TAG, "over apps: reshape", it) }
            At.ACTIVITY -> activity?.shape(e == FULL, e)
            At.SHELL -> {}
        }
    }

    private fun addWindow(): Boolean = try {
        wm.addView(this, params())
        true
    } catch (e: RuntimeException) {
        Log.w(TAG, "over apps: no window", e)
        false
    }

    private fun removeWindow() {
        if (at == At.WINDOW) runCatching { wm.removeViewImmediate(this) }.onFailure { Log.w(TAG, "over apps: remove", it) }
    }

    private fun params() = WindowManager.LayoutParams(
        MATCH,
        if (edge == FULL) MATCH else WRAP,
        WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
        // Never takes the keys (the app in front keeps them; r1d reads the wheel and button itself), and
        // touches outside the window go to the app.
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
            WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
        PixelFormat.TRANSLUCENT,
    ).apply {
        gravity = if (edge == FULL) Gravity.TOP else edge
        title = "r1os over apps"
        layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
    }

    companion object {
        private const val TAG = "r1os"
        private const val FULL = 0
        private const val MATCH = ViewGroup.LayoutParams.MATCH_PARENT
        private const val WRAP = ViewGroup.LayoutParams.WRAP_CONTENT

        /** The shell's dock, for [OverActivity] (there's one shell). */
        @SuppressLint("StaticFieldLeak")
        var current: Dock? = null
            private set
    }
}
