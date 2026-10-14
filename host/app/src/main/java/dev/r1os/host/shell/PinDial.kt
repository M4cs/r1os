package dev.r1os.host.shell

import android.animation.ObjectAnimator
import android.annotation.SuppressLint
import android.content.Context
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.animation.OvershootInterpolator
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject

/**
 * The 4-digit wheel lock (ADR-0016, Q28: "Like a 4 digit safe lock, vertical scroll on each, click to submit, double
 * click to go back"): four reels, the lit one rolled by the wheel (up is higher), its neighbours' digits showing above
 * and below as on a safe's dial. A click locks the digit in and lights the next; the fourth click hands the code to
 * [onCode]. A double click steps back a reel, and out from the first ([onOut]). A digit locked in shows as a dot, so
 * nobody reads it over your shoulder. A wrong code shakes the reels and clears them ([wrong]).
 */
@SuppressLint("ViewConstructor")
class PinDial(context: Context) : LinearLayout(context) {
    var onCode: (String) -> Unit = {}
    var onOut: () -> Unit = {}
    private val digits = IntArray(4)
    private var at = 0
    /** A code went to be checked: input waits for [wrong] or a [reset]. */
    private var busy = false
    private var title = "dial your pin"
    private val label = context.text(17f, 650, Theme.MUTED).apply { gravity = Gravity.CENTER }
    private val reels = List(4) { Reel(context) }
    private val row = LinearLayout(context).apply {
        orientation = HORIZONTAL
        gravity = Gravity.CENTER
        reels.forEachIndexed { i, r ->
            addView(r, LayoutParams(context.dpi(Reel.W), context.dpi(Reel.H)).apply { if (i > 0) leftMargin = context.dpi(10f) })
        }
    }
    private val hint = context.text(13f, 500, Theme.MUTED).apply {
        gravity = Gravity.CENTER
        text = "turn · click for the next · double click back"
        alpha = 0.7f
    }
    private val untitle = Runnable { label.text = title; label.setTextColor(Theme.MUTED) }

    init {
        orientation = VERTICAL
        gravity = Gravity.CENTER_HORIZONTAL
        addView(label, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
        addView(row, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(14f) })
        addView(hint, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(14f) })
        reset()
    }

    /** All back to 0, the first reel lit, `title` over them. */
    fun reset(title: String = this.title) {
        this.title = title
        removeCallbacks(untitle)
        label.text = title
        label.setTextColor(Theme.MUTED)
        busy = false
        at = 0
        digits.fill(0)
        reels.forEachIndexed { i, r -> if (i == 0) r.lit(0) else r.idle(0) }
    }

    /** The wheel, a click or a double click. True when it took it. */
    fun input(ev: Ev): Boolean {
        when {
            ev.type == "wheel" -> {
                if (busy) return true
                val up = -ev.int("delta").coerceIn(-1, 1)
                if (up == 0) return true
                digits[at] = Math.floorMod(digits[at] + up, 10)
                reels[at].roll(digits[at], up)
                Sounds.play(Sounds.S.TICK)
            }
            ev.isTap -> {
                if (busy) return true
                reels[at].entered()
                at++
                Sounds.play(Sounds.S.POP)
                if (at == 4) {
                    busy = true
                    onCode(digits.joinToString(""))
                } else {
                    reels[at].lit(digits[at])
                }
            }
            ev.isDoubleTap -> {
                if (busy) return true
                if (at == 0) { onOut(); return true }
                reels[at].idle(digits[at])
                at--
                reels[at].lit(digits[at])
                Sounds.play(Sounds.S.BACK)
            }
            else -> return false
        }
        return true
    }

    /** Not the code (`why`: r1d's words, "wrong code · wait 30 s"): the reels shake and clear. */
    fun wrong(why: String) {
        Sounds.play(Sounds.S.ERROR)
        reset()
        label.text = why
        label.setTextColor(Theme.BAD)
        ObjectAnimator.ofFloat(row, View.TRANSLATION_X, 0f, -context.dp(14f), context.dp(12f), -context.dp(9f), context.dp(6f), -context.dp(3f), 0f).apply {
            duration = 420
            start()
        }
        postDelayed(untitle, 2600)
    }
}

/** One reel: the digit big in a rounded slot, the ones either side of it faint above and below. */
@SuppressLint("ViewConstructor")
private class Reel(context: Context) : FrameLayout(context) {
    private val rowH = context.dp(ROW)
    /** Five rows (d+2 … d−2, higher above), the middle one in the slot. */
    private val cells = List(5) { i ->
        TextView(context).apply {
            setTextSize(TypedValue.COMPLEX_UNIT_DIP, 38f)
            typeface = Theme.font(750)
            fontFeatureSettings = "tnum"
            gravity = Gravity.CENTER
            includeFontPadding = false
            setTextColor(Theme.FG)
            alpha = when (i) { 2 -> 1f; 1, 3 -> 0.28f; else -> 0.1f }
        }
    }
    private val strip = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        cells.forEach { addView(it, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(ROW))) }
    }
    /** Where the strip sits with its middle row in the slot. */
    private val base = (context.dp(H) - 5 * rowH) / 2f

    init {
        clipChildren = true
        addView(strip, LayoutParams(LayoutParams.MATCH_PARENT, context.dpi(ROW) * 5))
        strip.translationY = base
    }

    private fun show(d: Int) {
        cells.forEachIndexed { i, c -> c.text = Math.floorMod(d + 2 - i, 10).toString() }
    }

    /** The one the wheel turns. */
    fun lit(d: Int) {
        show(d)
        background = roundRect(Theme.withAlpha(Theme.accent, 0.16f), context.dp(18f), context.dpi(2f), Theme.accent)
        strip.alpha = 1f
    }

    /** Not reached yet (or stepped back from). */
    fun idle(d: Int) {
        show(d)
        background = roundRect(Theme.CARD, context.dp(18f))
        strip.alpha = 0.35f
    }

    /** Locked in: a dot, so it can't be read. */
    fun entered() {
        cells.forEachIndexed { i, c -> c.text = if (i == 2) "•" else "" }
        background = roundRect(Theme.CARD, context.dp(18f))
        strip.alpha = 1f
    }

    /** Rolled one step (`dir`: 1 up to `d`, −1 down): the strip slides a row, the next digit coming into the slot. */
    fun roll(d: Int, dir: Int) {
        show(d)
        strip.animate().cancel()
        strip.translationY = base - dir * rowH
        strip.animate().translationY(base).setDuration(170).setInterpolator(OvershootInterpolator(1.3f)).start()
    }

    companion object {
        const val W = 60f
        const val H = 132f
        const val ROW = 44f
    }
}

/**
 * Settings › Lock (ADR-0016): a 4-digit PIN, dialled on the wheel each time the R1 wakes. Set it, change it, or turn it
 * off, each with the current one. Only here: not from a linked computer, nor by an agent.
 */
class LockScreen(shell: Shell) : ListPage(shell, "Lock") {
    private var set = false
    private var grace = 0L
    private var loaded = false

    init {
        list.gap(6f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/lock") { r ->
        set = r.optBoolean("set")
        grace = r.optLong("grace_s")
        loaded = true
        render()
    }

    override fun onShow() = load()
    override fun onReconnect() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "lock") load()
    }

    private fun render(): Unit = list.rebuild {
        if (!loaded) return@rebuild
        note("A 4-digit code you dial on the wheel, like a safe's, each time the R1 wakes. Until then nobody can talk to your agents or open anything; alarms and timers still ring.")
        if (!set) {
            item("lock-simple", "set a pin", accent) { shell.push(PinSetScreen(shell, "new")) }
        } else {
            item("lock-simple", "pin", accent, trailing = "on") {}
            // "Lock after" (Max, 2026-10-11): a quick sleep and wake doesn't ask; a click steps through the choices.
            item("timer", "lock after", sub = "how long it stays open once the screen's off", trailing = graceText(grace)) {
                val next = GRACES[(GRACES.indexOf(grace).coerceAtLeast(0) + 1) % GRACES.size]
                R1d.post("/api/lock/grace", JSONObject().put("grace_s", next)) { load() }
            }
            item("password", "change it") { shell.push(PinSetScreen(shell, "change")) }
            item("lock-simple-open", "turn it off") { shell.push(PinSetScreen(shell, "off")) }
        }
    }
}

/** r1d's "lock after" choices (s): right away, then a minute to an hour. */
private val GRACES = listOf(0L, 60L, 300L, 600L, 900L, 1800L, 3600L)

private fun graceText(s: Long): String = when {
    s == 0L -> "right away"
    s < 3600 -> "${s / 60} min"
    else -> "1 hour"
}

/** Dialling a new PIN twice (`new`), the current one then a new one twice (`change`), or the current one (`off`). */
class PinSetScreen(shell: Shell, private val mode: String) : PageScreen(shell, when (mode) { "new" -> "Set a PIN"; "change" -> "Change PIN"; else -> "Turn off PIN" }) {
    private val dial = PinDial(context)
    /** `current`, `new` or `again`. */
    private var step = if (mode == "new") "new" else "current"
    private var current: String? = null
    private var first: String? = null

    init {
        body.addView(dial, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.CENTER).apply {
            bottomMargin = context.dpi(30f)
        })
        dial.onOut = { shell.back() }
        dial.onCode = { code -> onCode(code) }
        dial.reset(if (step == "new") "a new pin" else "your pin")
    }

    private fun onCode(code: String) {
        when (step) {
            "current" -> if (mode == "off") {
                R1d.post("/api/lock/off", JSONObject().put("current", code), onError = { dial.wrong(it) }) {
                    shell.flash("pin off")
                    shell.back()
                }
            } else {
                R1d.post("/api/lock/check", JSONObject().put("pin", code), onError = { dial.wrong(it) }) {
                    current = code
                    step = "new"
                    dial.reset("a new pin")
                }
            }
            "new" -> {
                first = code
                step = "again"
                dial.reset("once more")
            }
            else -> if (code != first) {
                first = null
                step = "new"
                dial.reset("a new pin")
                dial.wrong("they didn't match")
            } else {
                val body = JSONObject().put("pin", code).apply { current?.let { put("current", it) } }
                R1d.put("/api/lock", body, onError = { dial.wrong(it) }) {
                    shell.flash("pin set")
                    shell.back()
                }
            }
        }
    }

    override fun onInput(ev: Ev): Boolean = dial.input(ev)
}
