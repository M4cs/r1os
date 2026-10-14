package dev.r1os.host.shell

import android.content.Context
import android.os.SystemClock
import android.util.TypedValue
import android.view.Gravity
import android.widget.Chronometer
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject
import java.util.Locale

/**
 * Tools › Timer (ADR-0050): **new timer** (one wheel of lengths; a click starts it), **pomodoro** (its lengths and rounds
 * on the wheel, then start: [PomodoroScreen]; or by talking: "a pomodoro with 50 minutes of focus"), and the timers
 * running, each with its countdown. A click on one opens it. "Set a timer for ten minutes" does the same from anywhere.
 */
class TimerScreen(shell: Shell) : ListPage(shell, "Timer") {
    private var timers: List<JSONObject> = emptyList()
    /** r1d's clock minus this one's, at the last load (the same device: ~0). */
    private var skew = 0L
    private var loaded = false

    init {
        list.gap(6f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/timers") { r ->
        val a = r.optJSONArray("timers")
        timers = (0 until (a?.length() ?: 0)).mapNotNull { a?.optJSONObject(it) }
        skew = r.optLong("now", System.currentTimeMillis()) - System.currentTimeMillis()
        loaded = true
        render()
    }

    override fun onShow() = load()

    /** r1d restarted: its timers may not be the ones on screen. */
    override fun onReconnect() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "timers_changed") load()
    }

    private fun render(): Unit = list.rebuild {
        if (!loaded) return@rebuild
        item("plus", "new timer", accent, sub = lengthText(TimerPickScreen.last(context) * 1000L)) { shell.push(TimerPickScreen(shell)) }
        if (timers.none { it.optString("kind") == "pomodoro" }) {
            item("timer", "pomodoro", sub = PomodoroScreen.Lengths.last(context).text()) { shell.push(PomodoroScreen(shell)) }
        }
        if (timers.isNotEmpty()) section("running")
        for (t in timers) {
            val ringing = t.has("ringing")
            item(if (t.optString("kind") == "pomodoro") "timer" else "hourglass-medium", t.optString("title"), if (ringing) accent else null,
                sub = stateText(context, t), trailingView = countdown(context, t, skew, 15f)) {
                shell.push(TimerPage(shell, t))
            }
        }
    }
}

/**
 * A new timer on the wheel (ADR-0050): one reel of lengths, from 10 seconds to 3 hours (up is longer), starting at
 * the last one used. A click starts it; a double click goes back.
 */
class TimerPickScreen(shell: Shell) : PageScreen(shell, "New timer") {
    private var i = LENGTHS.indexOf(last(context)).takeIf { it >= 0 } ?: LENGTHS.indexOf(300)
    private val big = context.text(80f, 750).apply {
        fontFeatureSettings = "tnum"
        gravity = Gravity.CENTER
    }
    private val ends = context.text(16f, 500, Theme.MUTED).apply { gravity = Gravity.CENTER }
    private val hint = context.text(14f, 500, Theme.MUTED).apply { gravity = Gravity.CENTER; text = "turn to set · click to start" }

    init {
        val col = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setPadding(0, 0, 0, context.dpi(40f))
            addView(big)
            addView(ends, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(10f) })
            addView(hint, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(18f) })
        }
        body.addView(col, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        render()
    }

    private fun render() {
        val s = LENGTHS[i]
        big.text = lengthClock(s * 1000L)
        ends.text = "ends ${whenText(context, System.currentTimeMillis() + s * 1000L)}"
    }

    override fun onInput(ev: Ev): Boolean {
        when {
            ev.type == "wheel" -> {
                // Up (a negative delta) is longer.
                val next = i - ev.int("delta").coerceIn(-1, 1)
                if (next !in LENGTHS.indices) {
                    Sounds.play(Sounds.S.BUMP)
                    return true
                }
                i = next
                Sounds.play(Sounds.S.TICK)
                render()
            }
            ev.isTap -> start()
            else -> return false
        }
        return true
    }

    private fun start() {
        Sounds.play(Sounds.S.POP)
        val s = LENGTHS[i]
        context.getSharedPreferences("r1os", Context.MODE_PRIVATE).edit().putInt(KEY, s).apply()
        R1d.post("/api/timers", JSONObject().put("seconds", s), onError = { hint.text = it }) { shell.back() }
    }

    companion object {
        private const val KEY = "timer_last"
        /** The lengths on the wheel, in seconds. */
        val LENGTHS = listOf(
            10, 15, 20, 30, 45, 60, 90, 120, 180, 240, 300, 360, 420, 480, 540, 600, 720, 900, 1200, 1500, 1800, 2400, 2700,
            3000, 3600, 4500, 5400, 6300, 7200, 9000, 10800,
        )

        /** The last length started here (5 minutes until one is). */
        fun last(context: Context): Int = context.getSharedPreferences("r1os", Context.MODE_PRIVATE).getInt(KEY, 300)
    }
}

/**
 * A pomodoro's lengths and rounds on the wheel (T-010), from the last set started here (25, 5 and 15 minutes, 4 rounds
 * at first): a click on one starts turning it (up is more), a click keeps it, a double click puts it back. The long break
 * comes after every 4th round, so it's only here with more than 4 rounds. **Start** sends them to r1d (ADR-0050).
 */
class PomodoroScreen(shell: Shell) : ListPage(shell, "Pomodoro") {
    private var set = Lengths.last(context)
    /** The one being turned, and what it was. */
    private var turning: Pair<String, Int>? = null
    private var msg: String? = null

    init {
        list.gap(6f)
        render()
    }

    private fun render(): Unit = list.rebuild {
        msg?.let { note(it) }
        item("play", "start", accent, sub = set.text()) { start() }.tag = "start"
        row("focus", "focus", "timer", "min")
        row("break", "breaks", "coffee", "min")
        if (set.rounds > EVERY) row("long", "long break", "armchair", "min", "after every ${EVERY}th round")
        row("rounds", "rounds", "repeat", null)
    }

    private fun FocusList.row(key: String, label: String, icon: String, unit: String?, sub: String? = null) {
        val on = turning?.first == key
        val v = set[key].let { if (unit != null) "$it $unit" else "$it" }
        item(icon, label, if (on) accent else null, sub = if (on) "turn to change · click to keep" else sub,
            trailing = if (on) "‹ $v ›" else v) { if (turning == null) { turning = key to set[key]; Sounds.play(Sounds.S.OPEN); render() } }.tag = key
    }

    override fun extra(ev: Ev): Boolean {
        val (key, was) = turning ?: return false
        when {
            ev.type == "wheel" -> {
                // Up (a negative delta) is more.
                val steps = STEPS.getValue(key)
                val i = steps.indexOf(set[key]).takeIf { it >= 0 } ?: steps.indexOfFirst { it >= set[key] }.coerceAtLeast(0)
                val next = i - ev.int("delta").coerceIn(-1, 1)
                if (next !in steps.indices) { Sounds.play(Sounds.S.BUMP); return true }
                Sounds.play(Sounds.S.TICK)
                set = set.with(key, steps[next])
            }
            ev.isTap -> { Sounds.play(Sounds.S.POP); turning = null }
            ev.isDoubleTap -> { Sounds.play(Sounds.S.BACK); set = set.with(key, was); turning = null }
            else -> return false
        }
        render()
        return true
    }

    private fun start() {
        Sounds.play(Sounds.S.POP)
        set.save(context)
        val body = JSONObject().put("pomodoro", true).put("focus", set.focus).put("short_break", set.short)
            .put("long_break", set.long).put("every", EVERY).put("rounds", set.rounds)
        R1d.post("/api/timers", body, onError = { msg = it; render() }) { r ->
            r.optJSONObject("timer")?.let {
                shell.push(TimerPage(shell, it))
                shell.remove(this@PomodoroScreen)
            }
        }
    }

    /** Minutes, and rounds. */
    data class Lengths(val focus: Int, val short: Int, val long: Int, val rounds: Int) {
        operator fun get(key: String) = when (key) { "focus" -> focus; "break" -> short; "long" -> long; else -> rounds }

        fun with(key: String, v: Int) = when (key) { "focus" -> copy(focus = v); "break" -> copy(short = v); "long" -> copy(long = v); else -> copy(rounds = v) }

        /** "25 min focus, 5 min breaks, 4 rounds" (the long break has its own row: one line is all there's room for). */
        fun text() = "$focus min focus, $short min breaks, ${if (rounds == 1) "1 round" else "$rounds rounds"}"

        fun save(context: Context) = context.getSharedPreferences("r1os", Context.MODE_PRIVATE).edit()
            .putString(KEY, "$focus,$short,$long,$rounds").apply()

        companion object {
            private const val KEY = "pomodoro_last"

            /** The last set started here; r1d's usual one (ADR-0050) until then. */
            fun last(context: Context): Lengths {
                val n = context.getSharedPreferences("r1os", Context.MODE_PRIVATE).getString(KEY, null)
                    ?.split(',')?.mapNotNull { it.toIntOrNull() }
                return if (n != null && n.size == 4) Lengths(n[0], n[1], n[2], n[3]) else Lengths(25, 5, 15, 4)
            }
        }
    }

    companion object {
        /** A long break after every this many rounds (r1d's usual). */
        const val EVERY = 4
        /** What the wheel goes through for each, in minutes (rounds: 1 to r1d's 12). */
        private val STEPS = mapOf(
            "focus" to listOf(5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 75, 90, 105, 120),
            "break" to listOf(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 15, 20, 25, 30),
            "long" to listOf(5, 10, 15, 20, 25, 30, 35, 40, 45, 60),
            "rounds" to (1..12).toList(),
        )
    }
}

/** One timer: its countdown, big; pause or resume, a minute more, skip (a pomodoro's phase), and cancel. */
class TimerPage(shell: Shell, private var timer: JSONObject) : ListPage(shell, timer.optString("title")) {
    private val timerId = timer.optString("id")
    private val confirm: Confirm = Confirm { render() }
    private var skew = 0L

    init {
        list.gap(6f)
        render()
    }

    private fun reload() = R1d.get("/api/timers") { r ->
        val a = r.optJSONArray("timers") ?: return@get
        val found = (0 until a.length()).mapNotNull { a.optJSONObject(it) }.firstOrNull { it.optString("id") == timerId }
        // Gone: this page goes, once (the cancel's answer and its timers_changed both get here).
        if (found == null) { shell.remove(this@TimerPage); return@get }
        skew = r.optLong("now", System.currentTimeMillis()) - System.currentTimeMillis()
        timer = found
        header.setTitle(timer.optString("title"))
        render()
    }

    override fun onShow() = reload()

    /** r1d restarted: the timer may be gone (its page goes back then). */
    override fun onReconnect() = reload()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "timers_changed") reload()
    }

    private fun render(): Unit = list.rebuild {
        val big = countdown(context, timer, skew, 64f)
        big.gravity = Gravity.CENTER
        place(big)
        note(stateText(context, timer), center = true)
        val running = timer.has("ends")
        val pomodoro = timer.optString("kind") == "pomodoro"
        if (timer.has("ringing")) {
            item("hand", "stop", accent) { op("stop") }
        } else {
            item(if (running) "pause" else "play", if (running) "pause" else "resume", accent) { op(if (running) "pause" else "resume") }
        }
        item("plus", "a minute more") { op("add", JSONObject().put("seconds", 60)) }
        if (pomodoro && !timer.has("ringing")) {
            val next = if (timer.optString("phase") == "focus") "the break" else "focus"
            item("skip-forward", "skip to $next") { op("skip") }
        }
        val armed = confirm.armed == "cancel"
        item("trash", if (armed) "Click again to cancel" else "cancel", danger = armed) {
            confirm.tap("cancel") { R1d.delete("/api/timers/${R1d.enc(timerId)}") { shell.remove(this@TimerPage) } }
        }
    }

    private fun op(name: String, body: JSONObject = JSONObject()) {
        Sounds.play(Sounds.S.POP)
        R1d.post("/api/timers/${R1d.enc(timerId)}/$name", body) { reload() }
    }
}

/**
 * A timer's countdown (ADR-0050): Android's Chronometer, which only ticks while its window is on screen (nothing runs
 * with the display off). A paused one shows what's left, still; at zero it stops at 0:00 until r1d says it rang.
 */
fun countdown(context: Context, t: JSONObject, skew: Long, size: Float): Chronometer = Chronometer(context).apply {
    setTextSize(TypedValue.COMPLEX_UNIT_DIP, size)
    typeface = Theme.font(if (size > 30f) 750 else 650)
    setTextColor(if (t.has("ringing")) Theme.accent else Theme.FG)
    includeFontPadding = false
    fontFeatureSettings = "tnum"
    isCountDown = true
    val left = when {
        t.has("ends") -> t.optLong("ends") - System.currentTimeMillis() - skew
        t.has("left") -> t.optLong("left")
        else -> 0L
    }
    base = SystemClock.elapsedRealtime() + left.coerceAtLeast(0L)
    setOnChronometerTickListener { c ->
        if (c.base <= SystemClock.elapsedRealtime()) {
            c.stop()
            c.text = "00:00"
        }
    }
    if (t.has("ends") && left > 0) start()
}

/** "focus · round 2 of 4", "paused · 4:12 left", "ends 5:40 pm", "time's up". */
fun stateText(context: Context, t: JSONObject): String {
    val parts = mutableListOf<String>()
    if (t.optString("kind") == "pomodoro" && !t.has("ringing")) {
        val phase = when (t.optString("phase")) { "focus" -> "focus"; "long" -> "long break"; else -> "break" }
        val rounds = t.optJSONObject("cycle")?.optInt("rounds") ?: 4
        parts += "$phase · round ${t.optInt("round", 1)} of $rounds"
    }
    parts += when {
        t.has("ringing") -> "time's up"
        t.has("left") -> "paused"
        t.has("ends") -> "ends ${whenText(context, t.optLong("ends"))}"
        else -> ""
    }
    return parts.filter { it.isNotEmpty() }.joinToString(" · ")
}

/** "10 min", "1 h 30 min", "45 s", "2 min 30 s" (r1d's own words for a length). */
fun lengthText(ms: Long): String {
    val s = (ms + 500) / 1000
    val (h, m, sec) = Triple(s / 3600, s % 3600 / 60, s % 60)
    return listOfNotNull(
        if (h > 0) "$h h" else null,
        if (m > 0) "$m min" else null,
        if (sec > 0 || (h == 0L && m == 0L)) "$sec s" else null,
    ).joinToString(" ")
}

/** A length as a clock: "5:00", "0:45", "1:30:00". */
fun lengthClock(ms: Long): String {
    val s = (ms + 500) / 1000
    return if (s >= 3600) String.format(Locale.ROOT, "%d:%02d:%02d", s / 3600, s % 3600 / 60, s % 60)
    else String.format(Locale.ROOT, "%d:%02d", s / 60, s % 60)
}
