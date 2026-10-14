package dev.r1os.host.shell

import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import org.json.JSONArray
import org.json.JSONObject

/**
 * What's up (R7): what's new and what's next, in one list, with no agent (r1d's `/api/whatsup`): replies you haven't
 * read, reminders, the next alarm, timers, today's routines, journal and recordings, notifications, what's playing.
 * Each row opens where it's from. A triple press can open it (Settings › Controls), and System One does for "what's up".
 */
class WhatsUpScreen(shell: Shell) : ListPage(shell, "What's up") {
    private var w: JSONObject? = null

    init {
        list.gap(6f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/whatsup") { w = it; render() }

    override fun onShow() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type in setOf("unread", "alarms_changed", "timers_changed", "routines_changed", "journal_changed")) load()
    }

    private fun arr(o: JSONObject?, k: String): List<JSONObject> = o?.optJSONArray(k).let { a -> (0 until (a?.length() ?: 0)).mapNotNull { a?.optJSONObject(it) } }

    private fun render(): Unit = list.rebuild {
        val w = w ?: run { note("…"); return@rebuild }
        var any = false
        val unread = arr(w, "unread")
        if (unread.isNotEmpty()) {
            section("for you")
            any = true
            for (u in unread) {
                val a = shell.agent(u.optString("agent"))
                item("chat-circle-dots", u.optString("name"), a?.let { dev.r1os.host.ui.Theme.colorOf(it.color) }, sub = u.optString("last").ifEmpty { u.optString("title") }, trailing = u.optInt("count").toString()) {
                    shell.open(Target.of("chat", "agent" to u.optString("agent"), "id" to u.optString("chat")))
                }
            }
        }
        // What came while the R1 slept (R8, crate::missed): by app, and your agent catches you up, reading more of each
        // through your connections. Only that marks them caught up.
        val missed = w.optJSONObject("missed")
        val apps = arr(missed, "apps")
        if (apps.isNotEmpty()) {
            section("while you were away")
            any = true
            for (m in apps) {
                val conn = m.optString("connection").takeIf { it.isNotEmpty() && !m.isNull("connection") }
                item("bell-simple", m.optString("app"), sub = listOfNotNull(m.optString("newest").ifEmpty { null }, conn?.let { "$it can read more" }).joinToString(" · "), trailing = m.optInt("count").toString()) {
                    shell.open(Target.of("tool", "id" to "notifications"))
                }
            }
            val a = shell.agent(shell.focus) ?: shell.agents.firstOrNull()
            if (a != null) {
                item("sparkle", "catch me up", dev.r1os.host.ui.Theme.colorOf(a.color), sub = "${a.name.lowercase()} reads them and says what matters") {
                    shell.open(Target.of("chat", "agent" to a.id, "say" to "What did I miss?"))
                }
            }
        }
        val rem = w.optJSONObject("reminders")
        val wentOff = arr(rem, "went_off")
        val today = arr(rem, "today")
        // Apps' reminders that went off while you were away (T-049): a click opens the app, and it's seen.
        val appReminders = arr(rem, "apps")
        if (wentOff.isNotEmpty() || today.isNotEmpty() || appReminders.isNotEmpty()) {
            section("reminders")
            any = true
            for (r in wentOff) item("bell-ringing", r.optString("text"), accent, sub = "went off") { shell.open(Target.of("tool", "id" to "reminders")) }
            for (r in appReminders) {
                val sub = listOf(r.optString("name"), whenText(context, r.optLong("at"))).filter { it.isNotEmpty() }.joinToString(" · ")
                item("bell-ringing", r.optString("title"), accent, sub = sub) {
                    R1d.post("/api/notices/${r.optLong("id")}/seen")
                    shell.open(Target.of("app", "id" to r.optString("app")))
                }
            }
            for (r in today) item("bell", r.optString("text"), sub = whenText(context, r.optLong("at"))) { shell.open(Target.of("tool", "id" to "reminders")) }
        }
        val alarm = w.optJSONObject("next_alarm")
        val timers = arr(w, "timers")
        val routines = arr(w, "routines")
        if (alarm != null || timers.isNotEmpty() || routines.isNotEmpty()) {
            section("next")
            any = true
            alarm?.let { a -> item("alarm", whenText(context, a.optLong("at")), sub = a.optString("label").ifEmpty { "alarm" }) { shell.open(Target.of("tool", "id" to "alarms")) } }
            for (t in timers) {
                val sub = when {
                    t.optBoolean("ringing") -> "time's up"
                    t.has("left") && !t.isNull("left") -> "paused"
                    else -> "ends ${whenText(context, t.optLong("ends"))}"
                }
                item("timer", t.optString("title"), sub = sub) { shell.open(Target.of("tool", "id" to "timer")) }
            }
            for (r in routines) item("repeat", r.optString("title"), sub = whenText(context, r.optLong("at"))) { shell.open(Target.of("tool", "id" to "routines")) }
        }
        val journal = w.optJSONObject("journal")
        val recs = w.optJSONObject("recordings")
        val notes = w.optJSONObject("notifications")
        val today2 = (journal?.optInt("count") ?: 0) + (recs?.optInt("count") ?: 0) + (notes?.optInt("count") ?: 0)
        if (today2 > 0) {
            section("today")
            any = true
            journal?.takeIf { it.optInt("count") > 0 }?.let { j ->
                item("notebook", "${j.optInt("count")} in your journal", sub = j.optString("last")) { shell.open(Target.of("tool", "id" to "journal")) }
            }
            recs?.takeIf { it.optInt("count") > 0 }?.let { r ->
                item("microphone", "${r.optInt("count")} recorded", sub = r.optString("last")) { shell.open(Target.of("tool", "id" to "recorder")) }
            }
            notes?.takeIf { it.optInt("count") > 0 }?.let { n ->
                val newest = n.optJSONObject("newest")
                item("bell-simple", "${n.optInt("count")} notification${if (n.optInt("count") == 1) "" else "s"}", sub = newest?.let { "${it.optString("app")}: ${it.optString("title")}" }) {
                    shell.open(Target.of("tool", "id" to "notifications"))
                }
            }
        }
        w.optJSONObject("playing")?.let { p ->
            section("playing")
            any = true
            item("music-notes", p.optString("title").ifEmpty { p.optString("app") }, sub = "${p.optString("app")}${if (p.optBoolean("playing")) "" else " · paused"}") {}
        }
        if (!any) note("all caught up: nothing new, nothing next.")
    }
}

/**
 * Tools › Routines (R7): an agent's turn on a schedule, made by talking ("every weekday at 7:30, tell me the weather and
 * my day") or from the companion. A click opens one: on or off, run it now, show it when done, its chat, remove it.
 */
class RoutinesScreen(shell: Shell) : ListPage(shell, "Routines") {
    private var routines: List<JSONObject> = emptyList()
    private var loaded = false

    init {
        list.gap(6f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/routines") { r ->
        val a = r.optJSONArray("routines") ?: JSONArray()
        routines = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
        loaded = true
        render()
    }

    override fun onShow() = load()
    override fun onReconnect() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "routines_changed") load()
    }

    private fun render(): Unit = list.rebuild {
        if (!loaded) return@rebuild
        note("Ask any agent: “every weekday at 7:30, tell me the weather and what's on my day.” It runs then, in your chat with that agent.")
        for (r in routines) {
            val on = r.optBoolean("on")
            item("repeat", r.optString("title"), if (on) accent else null, sub = routineWords(shell, r), trailing = if (on) "on" else "off") {
                shell.push(RoutinePage(shell, r))
            }
        }
    }
}

/** "7:30 am · weekdays · archer" (without the time, "weekdays · archer"). */
private fun routineWords(shell: Shell, r: JSONObject, time: Boolean = true): String =
    listOf(if (time) clockText(shell.context, r.optInt("minute")) else "", repeatsText(r.optInt("days")), shell.agent(r.optString("agent"))?.name.orEmpty())
        .filter { it.isNotEmpty() }.joinToString(" · ")

/** One routine: what it asks, on or off, run now, show it when done, its chat, remove. */
class RoutinePage(shell: Shell, private var r: JSONObject) : ListPage(shell, r.optString("title")) {
    private val routine = r.optString("id")
    private val confirm: Confirm = Confirm { render() }
    /** Running now: from a click on "run it now", or r1d's `awake` for it (on when it starts, off when it's done). */
    private var running = false

    init {
        list.gap(6f)
        render()
    }

    private fun reload() = R1d.get("/api/routines") { res ->
        val a = res.optJSONArray("routines") ?: return@get
        val found = (0 until a.length()).mapNotNull { a.optJSONObject(it) }.firstOrNull { it.optString("id") == routine }
        if (found == null) { shell.back(); return@get }
        r = found
        render()
    }

    override fun onShow() = reload()

    override fun onAnyEvent(ev: Ev) {
        when (ev.type) {
            "routines_changed" -> reload()
            "awake" -> if (ev.str("why") == "routine:$routine") {
                running = ev.bool("on")
                render()
            }
        }
    }

    private fun render(): Unit = list.rebuild {
        note("“${r.optString("prompt")}”", size = 16f)
        val on = r.optBoolean("on")
        val next = r.optLong("at").takeIf { on && it > 0 }?.let { "next ${whenText(context, it)} · ${routineWords(shell, r, time = false)}" }
        item("repeat", if (on) "on" else "off", if (on) accent else null, sub = next ?: routineWords(shell, r)) {
            R1d.post("/api/routines/${R1d.enc(routine)}/on", JSONObject().put("on", !on)) { reload() }
        }
        item("play", if (running) "running…" else "run it now", sub = "its reply goes to its chat") {
            if (running) return@item
            Sounds.play(Sounds.S.POP)
            running = true
            R1d.post("/api/routines/${R1d.enc(routine)}/run") { shell.flash("running ${r.optString("title").lowercase()}") }
            render()
        }
        val show = r.optBoolean("show")
        item("bell-ringing", "show it when done", if (show) accent else null, sub = "wakes the r1 and says so", trailing = if (show) "on" else "off") {
            R1d.post("/api/routines", JSONObject().put("id", routine).put("show", !show)) { reload() }
        }
        r.optString("chat").takeIf { it.isNotEmpty() }?.let { chat ->
            item("chat-circle", "its chat", sub = r.optLong("last").takeIf { it > 0 }?.let { "last ran ${whenText(context, it)}" }) {
                shell.open(Target.of("chat", "agent" to r.optString("agent"), "id" to chat))
            }
        }
        val armed = confirm.armed == "remove"
        item("trash", if (armed) "Click again to remove" else "remove", danger = armed) {
            confirm.tap("remove") { R1d.delete("/api/routines/${R1d.enc(routine)}") { shell.back() } }
        }
    }
}
