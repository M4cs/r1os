package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.util.TypedValue
import android.view.Gravity
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextClock
import android.widget.TextView
import dev.r1os.host.net.Agent
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Haptics
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text

/**
 * An alarm ringing (ADR-0028), or a timer (ADR-0050). r1d woke the display, and this fills it: the time, big;
 * the alarm's label (a timer's: "pasta", "10 min timer"); the agent that set it, bouncing; a looping tone that
 * grows louder ([Sounds.alarm]); and the motor pulsing with it ([Haptics.ring]). A click stops it, and a double
 * click snoozes an alarm ten minutes or gives a timer a minute more. Both buttons work by touch as well. It keeps
 * the display on while it rings, and gives up after ten minutes. It's in the shell's [Dock], so it rings over
 * other apps too.
 */
@SuppressLint("ViewConstructor")
class AlarmView(context: Context, private val onEnd: (id: String, snooze: Boolean, timer: Boolean) -> Unit) : FrameLayout(context) {
    private val character = CharacterView(context, context.dpi(120f))
    private val clock = TextClock(context).apply {
        format12Hour = "h:mm"
        format24Hour = "H:mm"
        setTextSize(TypedValue.COMPLEX_UNIT_DIP, 84f)
        typeface = Theme.font(700)
        setTextColor(Theme.FG)
        includeFontPadding = false
        fontFeatureSettings = "tnum"
        gravity = Gravity.CENTER
    }
    private val label = context.text(22f, 650, Theme.MUTED).apply { gravity = Gravity.CENTER; maxLines = 2 }
    private val stopButton = pill("stop", primary = true)
    private val snoozeButton = pill("snooze", primary = false)
    /** The ringing alarm's (or timer's) id; null when it isn't ringing. */
    var ringing: String? = null
        private set
    /** What's ringing is a timer. */
    private var timer = false
    private val giveUp = Runnable { ringing?.let { end(it, snooze = false) } }

    init {
        setBackgroundColor(Theme.BG)
        isClickable = true // swallows touches to what's underneath
        visibility = GONE
        val col = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            addView(character, LinearLayout.LayoutParams(context.dpi(120f), context.dpi(120f)))
            addView(clock, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(10f) })
            addView(label, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
        }
        addView(col, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT, Gravity.TOP).apply {
            topMargin = context.dpi(44f)
            leftMargin = context.dpi(20f)
            rightMargin = context.dpi(20f)
        })
        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(snoozeButton, LinearLayout.LayoutParams(0, context.dpi(64f), 1f).apply { rightMargin = context.dpi(10f) })
            addView(stopButton, LinearLayout.LayoutParams(0, context.dpi(64f), 1f))
        }
        addView(row, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT, Gravity.BOTTOM).apply {
            setMargins(context.dpi(16f), 0, context.dpi(16f), context.dpi(20f))
        })
        stopButton.setOnClickListener { ringing?.let { end(it, snooze = false) } }
        snoozeButton.setOnClickListener { ringing?.let { end(it, snooze = true) } }
    }

    private fun pill(word: String, primary: Boolean): TextView = context.text(22f, 750, if (primary) Theme.ON_ACCENT else Theme.FG).apply {
        text = word
        gravity = Gravity.CENTER
    }

    /** `id` rings: `label` (or just the time), with `a`'s face; a `timer`'s double click is a minute more. */
    fun ring(id: String, label: String, a: Agent, timer: Boolean = false) {
        val accent = Theme.colorOf(a.color)
        this.timer = timer
        this.label.text = label.ifBlank { if (timer) "time's up" else "alarm" }
        snoozeButton.text = if (timer) "+1 min" else "snooze"
        stopButton.background = roundRect(accent, context.dp(22f))
        snoozeButton.background = roundRect(Theme.CARD, context.dp(22f))
        character.set(a.form, a.color, a.finish, "speaking", eyes = a.eyes, wear = a.wear)
        character.setActive(true)
        val fresh = ringing == null
        ringing = id
        keepScreenOn = true
        visibility = VISIBLE
        if (fresh) {
            alpha = 0f
            animate().alpha(1f).setDuration(240).start()
            Sounds.alarm(context, true)
            Haptics.ring(true)
        }
        removeCallbacks(giveUp)
        postDelayed(giveUp, GIVE_UP_MS)
    }

    /** It stopped (here, by the agent, or elsewhere): quiet, and gone. */
    fun stopped(id: String? = null) {
        if (ringing == null || (id != null && id != ringing)) return
        ringing = null
        removeCallbacks(giveUp)
        Sounds.alarm(context, false)
        Haptics.ring(false)
        keepScreenOn = false
        character.setActive(false)
        animate().cancel()
        animate().alpha(0f).setDuration(200).withEndAction { visibility = GONE }.start()
    }

    /** A click (stop) or a double click (snooze, or a timer's minute more). */
    fun end(id: String, snooze: Boolean) {
        val wasTimer = timer
        stopped(id)
        onEnd(id, snooze, wasTimer)
    }

    private companion object {
        /** An alarm nobody stops gives up after this long. */
        const val GIVE_UP_MS = 10 * 60_000L
    }
}
