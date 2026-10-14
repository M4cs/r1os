package dev.r1os.host.shell

import android.content.Context
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
import dev.r1os.host.ui.text
import org.json.JSONObject
import java.util.Calendar
import java.util.Locale

/**
 * Tools › Alarms (ADR-0028): the R1's alarms. Talk to any agent to set them ("wake me at 7 on weekdays"), or set
 * one here with the wheel (**New alarm**). A click on an alarm opens it. Reminders and the to-do list have a
 * screen of their own (T-069: [RemindersScreen]); **reminders** here opens it.
 */
class AlarmsScreen(shell: Shell) : ListPage(shell, "Alarms") {
    private var alarms: List<JSONObject> = emptyList()
    private var loaded = false
    private val confirm: Confirm = Confirm { render() }

    init {
        list.gap(6f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/alarms") { r ->
        val a = r.optJSONArray("alarms")
        alarms = (0 until (a?.length() ?: 0)).mapNotNull { a?.optJSONObject(it) }
        loaded = true
        render()
    }

    override fun onShow() = load()

    /** r1d restarted: its alarms may not be the ones on screen. */
    override fun onReconnect() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "alarms_changed") load()
    }

    private fun render(): Unit = list.rebuild {
        if (!loaded) return@rebuild
        item("plus", "new alarm", accent) { shell.push(TimePickerScreen(shell, null)) }
        val (set, reminders) = alarms.partition { it.optString("kind") == "alarm" }
        if (set.isNotEmpty()) section("alarms")
        for (a in set) {
            val on = a.optBoolean("on")
            val sub = listOf(repeatsText(a.optInt("days")), a.optString("label")).filter { it.isNotBlank() }.joinToString(" · ")
            item("alarm", clockText(context, a.optInt("minute")), if (on) accent else null, sub = sub, trailing = if (on) "on" else "off") {
                shell.push(AlarmPage(shell, a))
            }
        }
        // Reminders and the to-do list (T-069).
        val todo = reminders.count { RemindersScreen.isTodo(it) && !it.has("done") }
        val timed = reminders.count { !RemindersScreen.isTodo(it) }
        section("")
        item("check-square", "reminders", sub = listOfNotNull(todo.takeIf { it > 0 }?.let { "$it to do" }, timed.takeIf { it > 0 }?.let { "$it set" }).joinToString(" · ").ifEmpty { "and your to-do list" }) {
            shell.push(RemindersScreen(shell))
        }
    }
}

/** One alarm: on or off, its time (the wheel picker), how it repeats, and removing it. */
class AlarmPage(shell: Shell, private var alarm: JSONObject) : ListPage(shell, clockText(shell.context, alarm.optInt("minute"))) {
    private val alarmId = alarm.optString("id")
    private val confirm: Confirm = Confirm { render() }

    init {
        list.gap(6f)
        render()
    }

    private fun reload() = R1d.get("/api/alarms") { r ->
        val a = r.optJSONArray("alarms") ?: return@get
        val found = (0 until a.length()).mapNotNull { a.optJSONObject(it) }.firstOrNull { it.optString("id") == alarmId }
        if (found == null) { shell.back(); return@get }
        alarm = found
        header.setTitle(clockText(context, alarm.optInt("minute")))
        render()
    }

    override fun onShow() = reload()

    /** r1d restarted: the alarm may be gone (its page goes back then). */
    override fun onReconnect() = reload()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "alarms_changed") reload()
    }

    private fun render(): Unit = list.rebuild {
        val on = alarm.optBoolean("on")
        alarm.optString("label").takeIf { it.isNotBlank() }?.let { note(it) }
        item("alarm", if (on) "on" else "off", if (on) accent else null, sub = alarm.optLong("at").takeIf { on && it > 0 }?.let { "rings ${whenText(context, it)}" }) {
            R1d.post("/api/alarms/${R1d.enc(alarmId)}/on", JSONObject().put("on", !on)) { reload() }
        }
        item("clock", "time", trailing = clockText(context, alarm.optInt("minute"))) { shell.push(TimePickerScreen(shell, alarm)) }
        item("repeat", "repeat", trailing = repeatsText(alarm.optInt("days"))) {
            // The common ones in turn; days of your own are set by talking ("on Mondays and Thursdays").
            val next = REPEATS[(REPEATS.indexOf(alarm.optInt("days") and 0x7f) + 1) % REPEATS.size]
            save(alarm.optInt("minute"), next)
        }
        val armed = confirm.armed == "remove"
        item("trash", if (armed) "Click again to remove" else "remove", danger = armed) {
            confirm.tap("remove") { R1d.delete("/api/alarms/${R1d.enc(alarmId)}") { shell.back() } }
        }
    }

    private fun save(minute: Int, days: Int) =
        R1d.post("/api/alarms", alarmBody(alarmId, minute, days, alarm.optString("label"))) { reload() }
}

/**
 * Setting an alarm with the wheel (Q33): the hour, then the minutes, then how it repeats. The wheel turns
 * the one that's lit (up is later), a click moves on, and the last click sets it. A double click steps
 * back, and out from the hour. A quick spin moves the minutes five at a time.
 */
class TimePickerScreen(shell: Shell, private val alarm: JSONObject?) : PageScreen(shell, if (alarm == null) "New alarm" else "Time") {
    private var minute = alarm?.optInt("minute") ?: (7 * 60)
    private var days = alarm?.optInt("days") ?: 0
    /** 0: the hour, 1: the minutes, 2: how it repeats. */
    private var field = 0
    private var lastTick = 0L
    private val twentyFour = DateFormat.is24HourFormat(context)

    private val hour = digits()
    private val colon = context.text(64f, 700, Theme.MUTED).apply { text = ":" }
    private val mins = digits()
    private val half = context.text(26f, 700, Theme.MUTED).apply { setPadding(context.dpi(8f), 0, 0, 0) }
    private val repeat = context.text(22f, 700).apply {
        gravity = Gravity.CENTER
        setPadding(context.dpi(18f), context.dpi(8f), context.dpi(18f), context.dpi(8f))
    }
    private val rings = context.text(15f, 500, Theme.MUTED).apply { gravity = Gravity.CENTER }

    init {
        val time = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER
            addView(hour)
            addView(colon)
            addView(mins)
            if (!twentyFour) addView(half)
        }
        val col = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setPadding(0, 0, 0, context.dpi(40f))
            addView(time)
            addView(repeat, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(18f) })
            addView(rings, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(14f) })
        }
        body.addView(col, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        render()
    }

    private fun digits() = context.text(72f, 750).apply {
        fontFeatureSettings = "tnum"
        gravity = Gravity.CENTER
        setPadding(context.dpi(10f), 0, context.dpi(10f), 0)
    }

    private fun render() {
        val h = minute / 60
        hour.text = if (twentyFour) "$h" else "${if (h % 12 == 0) 12 else h % 12}"
        mins.text = String.format(Locale.ROOT, "%02d", minute % 60)
        half.text = if (h < 12) "am" else "pm"
        repeat.text = repeatsText(days)
        rings.text = "rings ${whenText(context, nextRing())}"
        val lit = listOf(hour, mins, repeat)
        lit.forEachIndexed { i, v ->
            val on = i == field
            v.background = if (on) roundRect(LIT, context.dp(18f)) else null
            v.setTextColor(if (on) Theme.ON_ACCENT else Theme.FG)
        }
    }

    /** When it would ring next, as set now (shown under it). */
    private fun nextRing(): Long {
        val c = Calendar.getInstance()
        val now = c.timeInMillis
        c.set(Calendar.HOUR_OF_DAY, minute / 60)
        c.set(Calendar.MINUTE, minute % 60)
        c.set(Calendar.SECOND, 0)
        c.set(Calendar.MILLISECOND, 0)
        for (d in 0..7) {
            // Calendar's Monday is 2 … Sunday 1; r1d's Monday is bit 0.
            val bit = (c.get(Calendar.DAY_OF_WEEK) + 5) % 7
            if (c.timeInMillis > now && (days and 0x7f == 0 || days and (1 shl bit) != 0)) return c.timeInMillis
            c.add(Calendar.DAY_OF_MONTH, 1)
        }
        return c.timeInMillis
    }

    override fun onInput(ev: Ev): Boolean {
        when {
            ev.type == "wheel" -> {
                val now = SystemClock.uptimeMillis()
                val quick = now - lastTick < QUICK_MS
                lastTick = now
                // Up (a negative delta) is later.
                val up = -ev.int("delta")
                when (field) {
                    0 -> minute = Math.floorMod(minute + up * 60, DAY_MIN)
                    1 -> minute = (minute / 60) * 60 + Math.floorMod(minute % 60 + up * (if (quick) 5 else 1), 60)
                    else -> days = REPEATS[Math.floorMod(REPEATS.indexOf(days and 0x7f).coerceAtLeast(0) + up, REPEATS.size)]
                }
                Sounds.play(Sounds.S.TICK)
                render()
            }
            ev.isTap -> {
                if (field < 2) {
                    field++
                    Sounds.play(Sounds.S.POP)
                    render()
                } else {
                    save()
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
        Sounds.play(Sounds.S.POP)
        R1d.post("/api/alarms", alarmBody(alarm?.optString("id"), minute, days, alarm?.optString("label").orEmpty()), onError = { rings.text = it }) {
            shell.back()
        }
    }

    private companion object {
        const val DAY_MIN = 24 * 60
        /** Ticks closer than this are a spin: the minutes go five at a time. */
        const val QUICK_MS = 70L
        /** The lit field's fill: the shell's accent, like a focused row. */
        val LIT get() = Theme.accent
    }
}

/** How an alarm can repeat from the picker: once, every day, weekdays, weekends (r1d's day bits). */
private val REPEATS = listOf(0, 0x7f, 0x1f, 0x60)

/** The request that sets an alarm (`id`: changes that one). */
private fun alarmBody(id: String?, minute: Int, days: Int, label: String): JSONObject {
    val names = listOf("mon", "tue", "wed", "thu", "fri", "sat", "sun").filterIndexed { i, _ -> days and (1 shl i) != 0 }
    return JSONObject()
        .put("kind", "alarm")
        .put("time", String.format(Locale.ROOT, "%02d:%02d", minute / 60, minute % 60))
        .put("days", org.json.JSONArray(names))
        .put("label", label)
        .apply { if (!id.isNullOrEmpty()) put("id", id) }
}

/** A time of day, `minute` after midnight: "7:30 am" or "19:30", as the R1 is set. */
fun clockText(context: Context, minute: Int): String {
    val h = minute / 60
    val m = minute % 60
    if (DateFormat.is24HourFormat(context)) return String.format(Locale.ROOT, "%d:%02d", h, m)
    return String.format(Locale.ROOT, "%d:%02d %s", if (h % 12 == 0) 12 else h % 12, m, if (h < 12) "am" else "pm")
}

/** "once", "every day", "weekdays", "weekends", "mon, wed". Monday is bit 0 (r1d's alarms). */
fun repeatsText(days: Int): String = when (days and 0x7f) {
    0 -> "once"
    0x7f -> "every day"
    0x1f -> "weekdays"
    0x60 -> "weekends"
    else -> listOf("mon", "tue", "wed", "thu", "fri", "sat", "sun").filterIndexed { i, _ -> days and (1 shl i) != 0 }.joinToString(", ")
}

/** When `ms` is, from now: "5:00 pm", "tomorrow 7:00 am", "sat 5:00 pm", "oct 12 5:00 pm". */
fun whenText(context: Context, ms: Long): String {
    val c = Calendar.getInstance().apply { timeInMillis = ms }
    val time = clockText(context, c.get(Calendar.HOUR_OF_DAY) * 60 + c.get(Calendar.MINUTE))
    val today = Calendar.getInstance()
    val days = dayNumber(c) - dayNumber(today)
    return when {
        days == 0 -> time
        days == 1 -> "tomorrow $time"
        days == -1 -> "yesterday $time"
        days in 2..6 -> "${DateFormat.format("EEE", c).toString().lowercase()} $time"
        else -> "${DateFormat.format("MMM d", c).toString().lowercase()} $time"
    }
}

private fun dayNumber(c: Calendar): Int = c.get(Calendar.YEAR) * 400 + c.get(Calendar.DAY_OF_YEAR)
