package dev.r1os.host.shell

import android.os.SystemClock
import android.text.format.DateFormat
import android.view.Gravity
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject
import java.util.Calendar
import java.util.Locale

/**
 * Tools › Reminders (T-069; ADR-0028): your to-do list and your reminders. A to-do has no time: a click ticks it off (and
 * on again), and ticked ones go a day later or with **clear done**. A reminder goes off at its time, once or on chosen
 * days ("every weekday at 9"); a click arms removing it and a second removes it. **New reminder**: hold the button and say
 * what, then turn the wheel for when. Or tell any agent ("add milk to my list", "remind me every morning at 8 to stretch").
 */
class RemindersScreen(shell: Shell) : ListPage(shell, "Reminders") {
    private var reminders: List<JSONObject> = emptyList()
    private var loaded = false
    private val confirm: Confirm = Confirm { render() }

    init {
        list.gap(6f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/alarms") { r ->
        val a = r.optJSONArray("alarms")
        reminders = (0 until (a?.length() ?: 0)).mapNotNull { a?.optJSONObject(it) }.filter { it.optString("kind") == "reminder" }
        loaded = true
        render()
    }

    override fun onShow() = load()

    override fun onReconnect() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "alarms_changed") load()
    }

    private fun render(): Unit = list.rebuild {
        if (!loaded) return@rebuild
        item("plus", "new reminder", accent) { shell.push(NewReminderScreen(shell)) }
        val (todos, timed) = reminders.partition { isTodo(it) }
        val (done, open) = todos.partition { it.has("done") }
        if (todos.isNotEmpty()) section("to do")
        for (t in open + done) {
            val ticked = t.has("done")
            item(if (ticked) "check-square" else "square", t.optString("label"), if (ticked) null else accent, sub = if (ticked) "done" else null) {
                Sounds.play(if (ticked) Sounds.S.TICK else Sounds.S.POP)
                R1d.post("/api/alarms/${R1d.enc(t.optString("id"))}/done", JSONObject().put("done", !ticked)) { load() }
            }
        }
        if (done.isNotEmpty()) item("broom", "clear done", sub = "${done.size} ticked off") { R1d.post("/api/alarms/clear_done", JSONObject()) { load() } }
        if (timed.isNotEmpty()) section("reminders")
        for (r in timed) {
            val id = r.optString("id")
            val armed = confirm.armed == id
            val repeats = r.has("minute") && r.optInt("days") != 0
            val sub = when {
                r.has("fired") && !repeats -> "went off ${whenText(context, r.optLong("fired"))}"
                repeats -> "${repeatsText(r.optInt("days"))} ${clockText(context, r.optInt("minute"))}"
                else -> whenText(context, r.optLong("at"))
            }
            item(if (repeats) "repeat" else "bell", if (armed) "Click again to remove" else r.optString("label"), sub = sub, danger = armed) {
                confirm.tap(id) { R1d.delete("/api/alarms/${R1d.enc(id)}") { load() } }
            }
        }
        if (reminders.isEmpty()) note("Nothing yet. Add one here, or hold the button and tell an agent: “add milk to my list”, “remind me every weekday at 9 to take my vitamins”.", size = 14f)
    }

    companion object {
        /** A reminder with no time: on the to-do list. */
        fun isTodo(r: JSONObject) = r.optString("kind") == "reminder" && !r.has("at") && !r.has("fired") && !r.has("minute")
    }
}

/**
 * A new reminder on the R1 (T-069): hold the button and say what, then the wheel picks when: no time (a to-do), today,
 * tomorrow, a day this week, or every day, weekdays or weekends; then the hour and the minutes. A click moves on, and the
 * last one sets it; a double click steps back. Saying it again changes the words.
 */
class NewReminderScreen(shell: Shell) : PageScreen(shell, "New reminder") {
    private var words: String? = null
    /** 0: when, 1: the hour, 2: the minutes. */
    private var field = 0
    private var day = 0
    private var minute = run { val c = Calendar.getInstance(); ((c.get(Calendar.HOUR_OF_DAY) + 1) % 24) * 60 }
    private var lastTick = 0L
    private val twentyFour = DateFormat.is24HourFormat(context)
    private val days: List<Pair<String, Choice>> = choices()

    private val what = context.text(24f, 750).keepCase().apply {
        gravity = Gravity.CENTER
        setPadding(context.dpi(18f), 0, context.dpi(18f), 0)
    }
    private val whenView = context.text(22f, 700).apply {
        gravity = Gravity.CENTER
        setPadding(context.dpi(18f), context.dpi(8f), context.dpi(18f), context.dpi(8f))
    }
    private val hour = digits()
    private val colon = context.text(52f, 700, Theme.MUTED).apply { text = ":" }
    private val mins = digits()
    private val half = context.text(22f, 700, Theme.MUTED).apply { setPadding(context.dpi(6f), 0, 0, 0) }
    private val time = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER
        addView(hour)
        addView(colon)
        addView(mins)
        if (!twentyFour) addView(half)
    }
    private val hint = context.text(15f, 500, Theme.MUTED).apply { gravity = Gravity.CENTER }

    init {
        val col = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setPadding(0, 0, 0, context.dpi(30f))
            addView(what)
            addView(whenView, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(22f) })
            addView(time, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(10f) })
            addView(hint, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(16f) })
        }
        body.addView(col, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        render()
    }

    private fun digits() = context.text(56f, 750).apply {
        fontFeatureSettings = "tnum"
        gravity = Gravity.CENTER
        setPadding(context.dpi(8f), 0, context.dpi(8f), 0)
    }

    private sealed interface Choice {
        data object Todo : Choice
        data class On(val daysFromNow: Int) : Choice
        data class Every(val bits: Int) : Choice
    }

    /** No time, today, tomorrow, the next five days by name, then every day, weekdays, weekends. */
    private fun choices(): List<Pair<String, Choice>> {
        val out = mutableListOf<Pair<String, Choice>>("no time · to do" to Choice.Todo, "today" to Choice.On(0), "tomorrow" to Choice.On(1))
        val c = Calendar.getInstance()
        for (d in 2..6) {
            c.timeInMillis = System.currentTimeMillis()
            c.add(Calendar.DAY_OF_MONTH, d)
            out += DateFormat.format("EEEE", c).toString().lowercase() to Choice.On(d)
        }
        out += "every day" to Choice.Every(0x7f)
        out += "weekdays" to Choice.Every(0x1f)
        out += "weekends" to Choice.Every(0x60)
        return out
    }

    /** Your words (the shell hands them here): what to remember. */
    fun hear(text: String) {
        words = text.trim().trimEnd('.').replaceFirstChar { it.uppercase() }
        Sounds.play(Sounds.S.POP)
        render()
    }

    private fun render() {
        val w = words
        what.text = w ?: "hold the button and say what to remember"
        what.setTextColor(if (w == null) Theme.MUTED else Theme.FG)
        val choice = days[day].second
        whenView.text = days[day].first
        val timed = choice != Choice.Todo
        time.visibility = if (w != null && timed) android.view.View.VISIBLE else android.view.View.GONE
        whenView.visibility = if (w != null) android.view.View.VISIBLE else android.view.View.GONE
        val h = minute / 60
        hour.text = if (twentyFour) "$h" else "${if (h % 12 == 0) 12 else h % 12}"
        mins.text = String.format(Locale.ROOT, "%02d", minute % 60)
        half.text = if (h < 12) "am" else "pm"
        listOf(whenView, hour, mins).forEachIndexed { i, v ->
            val on = w != null && i == field
            v.background = if (on) roundRect(Theme.accent, context.dp(18f)) else null
            v.setTextColor(if (on) Theme.ON_ACCENT else Theme.FG)
        }
        if (hint.tag == null) {
            hint.text = when {
                w == null -> ""
                field == 0 && !timed -> "click to add it to your list"
                field == 0 -> "turn for when · click for the time"
                field == 1 -> "the hour · click for the minutes"
                else -> "the minutes · click to set it"
            }
        }
    }

    override fun onInput(ev: Ev): Boolean {
        if (words == null) return ev.isTap || ev.type == "wheel"
        when {
            ev.type == "wheel" -> {
                hint.tag = null
                val now = SystemClock.uptimeMillis()
                val quick = now - lastTick < 70L
                lastTick = now
                val up = -ev.int("delta")
                when (field) {
                    0 -> day = Math.floorMod(day + up, days.size)
                    1 -> minute = Math.floorMod(minute + up * 60, 24 * 60)
                    else -> minute = (minute / 60) * 60 + Math.floorMod(minute % 60 + up * (if (quick) 5 else 1), 60)
                }
                Sounds.play(Sounds.S.TICK)
                render()
            }
            ev.isTap -> {
                hint.tag = null
                if (field == 0 && days[day].second == Choice.Todo || field == 2) save() else {
                    field++
                    Sounds.play(Sounds.S.POP)
                    render()
                }
            }
            ev.isDoubleTap -> {
                if (field == 0) return false
                field--
                Sounds.play(Sounds.S.BACK)
                render()
            }
            else -> return false
        }
        return true
    }

    private fun save() {
        val w = words ?: return
        val body = JSONObject().put("kind", "reminder").put("text", w)
        val hhmm = String.format(Locale.ROOT, "%02d:%02d", minute / 60, minute % 60)
        when (val c = days[day].second) {
            Choice.Todo -> {}
            is Choice.On -> {
                val cal = Calendar.getInstance()
                cal.add(Calendar.DAY_OF_MONTH, c.daysFromNow)
                body.put("at", "${DateFormat.format("yyyy-MM-dd", cal)} $hhmm")
            }
            is Choice.Every -> {
                val names = listOf("mon", "tue", "wed", "thu", "fri", "sat", "sun").filterIndexed { i, _ -> c.bits and (1 shl i) != 0 }
                body.put("time", hhmm).put("days", JSONArray(names))
            }
        }
        Sounds.play(Sounds.S.POP)
        R1d.post("/api/alarms", body, onError = { hint.tag = "error"; hint.text = it.lowercase() }) {
            shell.flash(if (days[day].second == Choice.Todo) "on your list" else "reminder set")
            shell.back()
        }
    }
}
