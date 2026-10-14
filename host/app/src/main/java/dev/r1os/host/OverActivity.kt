package dev.r1os.host

import android.app.Activity
import android.os.Bundle
import android.view.Gravity
import android.view.KeyEvent
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowManager
import dev.r1os.host.shell.Dock

/**
 * The shell's talking, volume, reply and reminder layer ([Dock]) over another app, where Android hides
 * overlay windows (ADR-0027). Its own settings and Files set `HIDE_NON_SYSTEM_OVERLAY_WINDOWS`, but an
 * activity on top isn't an overlay. This one is see-through, so the app stays on screen underneath.
 *
 * The [Dock] starts it and moves its layer in. With only a card showing, the window shrinks to the card
 * and stops taking keys or touches outside it, so the app underneath works as usual. It finishes once
 * the layer has nothing to show, or r1os comes forward.
 */
class OverActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val dock = Dock.current
        if (dock == null) {
            finishNow()
            return
        }
        dock.attach(this)
        // After the content: the window has no decor (and no insets controller) before it.
        window.decorView.windowInsetsController?.hide(WindowInsets.Type.systemBars())
    }

    /** The layer's shape: the whole screen (`full`), else only the card at `edge` (`Gravity.TOP`/`BOTTOM`). */
    fun shape(full: Boolean, edge: Int) {
        window.setLayout(ViewGroup.LayoutParams.MATCH_PARENT, if (full) ViewGroup.LayoutParams.MATCH_PARENT else ViewGroup.LayoutParams.WRAP_CONTENT)
        window.setGravity(if (full) Gravity.TOP else edge)
        // A card leaves the app underneath its keys and touches; talking and the volume take them.
        val passing = WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL
        if (full) window.clearFlags(passing) else window.addFlags(passing)
    }

    /** Gone without an animation. */
    fun finishNow() {
        finish()
        @Suppress("DEPRECATION")
        overridePendingTransition(0, 0)
    }

    override fun onDestroy() {
        Dock.current?.detached(this)
        super.onDestroy()
    }

    // The wheel, the side button and Back: r1d delivers them to the shell (as for MainActivity).
    override fun dispatchKeyEvent(event: KeyEvent): Boolean = when (event.keyCode) {
        KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_BUTTON_1, KeyEvent.KEYCODE_BACK -> true
        else -> super.dispatchKeyEvent(event)
    }
}
