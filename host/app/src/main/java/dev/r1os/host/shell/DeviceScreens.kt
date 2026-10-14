package dev.r1os.host.shell

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.DeviceSettings
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject

/**
 * Settings › Display & sound (ADR-0045): brightness (or automatic), how long until the screen sleeps, the volume, and
 * r1os's sounds. A click on brightness or volume starts turning it with the wheel, another stops. Agents change the
 * same settings, with your approval; what they change shows here.
 */
class DisplayScreen(shell: Shell) : ListPage(shell, "Display & sound") {
    private var s: JSONObject = DeviceSettings.read(context)
    /** The level being turned: brightness or volume; null for none. */
    private var turning: String? = null
    private var lastTick = 0L
    private val levels = HashMap<String, Pair<LevelBar, android.widget.TextView>>()

    init {
        render()
    }

    override fun onShow() {
        s = DeviceSettings.read(context)
        render()
    }

    /** An agent's change (`device_set`), made by the shell: shown once it's in. */
    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "device_set") list.post { s = DeviceSettings.read(context); render() }
    }

    private fun render() {
        val keep = list.focus
        list.rebuild {
            val auto = s.optBoolean("auto_brightness")
            level("brightness", "sun", if (auto) "automatic: a turn sets it yourself" else null)
            item("sun-dim", "automatic brightness", if (auto) accent else null, sub = "follows the light around it", trailing = if (auto) "on" else "off") {
                set("auto_brightness", !auto)
            }
            val docked = s.optBoolean("docked")
            item("bed", "docked mode", if (docked) accent else null, sub = "on its side, the glance screen turns; while it charges, it stays on", trailing = if (docked) "on" else "off") {
                set("docked", !docked)
            }
            section("screen sleeps after")
            val now = s.optInt("screen_timeout")
            for (t in DeviceSettings.TIMEOUTS) {
                val on = t == now
                item(if (on) "check-circle" else "circle", timeoutWords(t), if (on) accent else null) { set("screen_timeout", t) }
            }
            section("sound")
            level("volume", "speaker-high", null)
            val sounds = s.optBoolean("sounds", true)
            item(if (sounds) "speaker-high" else "speaker-slash", "r1os sounds", if (sounds) accent else null, sub = "clicks and ticks as you use it", trailing = if (sounds) "on" else "off") {
                set("sounds", !sounds)
            }
            // A touch from the motor with each of them (Haptics), however strong you like it.
            section("haptics")
            val haptics = s.optString("haptics", "medium")
            for ((lv, sub) in HAPTICS) {
                val on = lv == haptics
                item(if (on) "check-circle" else "circle", lv, if (on) accent else null, sub = sub) { set("haptics", lv) }
            }
            if (haptics != "off") {
                val wheel = s.optBoolean("haptics_wheel", true)
                item(if (wheel) "vibrate" else "circle", "on the wheel too", if (wheel) accent else null, sub = "a tick as you turn it", trailing = if (wheel) "on" else "off") {
                    set("haptics_wheel", !wheel)
                }
            }
        }
        if (keep >= 0) list.setFocus(keep.coerceAtMost(list.size - 1), animate = false)
    }

    /** A level the wheel turns: its name and value, and a bar. */
    private fun FocusList.level(key: String, icon: String, sub: String?) {
        val label = context.text(19f, 700).apply { text = key }
        val value = context.text(14f, 700, Theme.MUTED)
        val bar = LevelBar(context)
        val glyph = Icons.drawable(icon, Theme.ICON)
        val tile = FrameLayout(context).apply {
            background = roundRect(Theme.withAlpha(Theme.ICON, 0.18f), context.dp(12f))
            addView(ImageView(context).apply { setImageDrawable(glyph) }, FrameLayout.LayoutParams(context.dpi(20f), context.dpi(20f), Gravity.CENTER))
        }
        val tileRest = tile.background
        val subView = sub?.let { context.text(14f, 500, Theme.MUTED).apply { text = it } }
        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(context.dpi(10f), context.dpi(10f), context.dpi(12f), context.dpi(12f))
            addView(tile, LinearLayout.LayoutParams(context.dpi(38f), context.dpi(38f)).apply { rightMargin = context.dpi(12f) })
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    addView(label, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                    addView(value)
                })
                if (subView != null) addView(subView)
                addView(bar, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(12f)).apply { topMargin = context.dpi(7f) })
            }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        }
        place(row)
        register(row, onFocus = { on ->
            label.setTextColor(if (on) Theme.ON_ACCENT else Theme.FG)
            value.setTextColor(if (on) Theme.ON_ACCENT else Theme.MUTED)
            subView?.setTextColor(if (on) Theme.withAlpha(Theme.ON_ACCENT, 0.7f) else Theme.MUTED)
            glyph.setColor(if (on) Theme.ON_ACCENT else Theme.ICON)
            tile.background = if (on) roundRect(Theme.withAlpha(android.graphics.Color.BLACK, 0.12f), context.dp(12f)) else tileRest
            bar.ink = on
        }) {
            turning = if (turning == key) null else key
            showLevel(key)
        }
        levels[key] = bar to value
        showLevel(key)
    }

    private fun showLevel(key: String) {
        val (bar, value) = levels[key] ?: return
        val v = s.optInt(key)
        bar.set(v / 100f, list.accent)
        value.text = if (turning == key) "$v% · turning" else "$v%"
    }

    /** While a level is being turned, the wheel changes it (5% a tick; more when it's spun fast). */
    override fun extra(ev: Ev): Boolean {
        val key = turning ?: return false
        if (ev.type != "wheel") return false
        val now = android.os.SystemClock.uptimeMillis()
        val step = if (now - lastTick < FAST_MS) 10 else 5
        lastTick = now
        val d = if (ev.int("delta") > 0) -1 else 1
        val v = (s.optInt(key) + d * step).coerceIn(0, 100)
        if (v == s.optInt(key)) { Sounds.play(Sounds.S.BUMP); return true }
        s.put(key, v)
        val change = JSONObject().put(key, v)
        // Turning brightness yourself: it stops being automatic.
        if (key == "brightness" && s.optBoolean("auto_brightness")) {
            s.put("auto_brightness", false)
            change.put("auto_brightness", false)
            DeviceSettings.apply(context, change)
            render()
            return true
        }
        DeviceSettings.apply(context, change)
        if (key == "volume") Sounds.level(v, 100) else Sounds.play(Sounds.S.TICK)
        showLevel(key)
        return true
    }

    private fun set(key: String, v: Any) {
        s.put(key, v)
        DeviceSettings.apply(context, JSONObject().put(key, v))
        render()
    }

    /** A level's bar: the track, and how far it's filled. Ink when its row is focused (the row is the accent then). */
    private class LevelBar(context: Context) : View(context) {
        private val track = Paint(Paint.ANTI_ALIAS_FLAG)
        private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
        private var at = 0f
        private var color = Theme.accent
        var ink = false
            set(v) { field = v; invalidate() }

        fun set(f: Float, c: Int) {
            at = f.coerceIn(0f, 1f)
            color = c
            invalidate()
        }

        override fun onDraw(c: Canvas) {
            val w = width.toFloat()
            val h = height.toFloat()
            track.color = if (ink) Theme.withAlpha(Theme.ON_ACCENT, 0.25f) else Theme.FOCUS
            fill.color = if (ink) Theme.ON_ACCENT else color
            c.drawRoundRect(0f, 0f, w, h, h / 2, h / 2, track)
            if (at > 0f) c.drawRoundRect(0f, 0f, maxOf(h, w * at), h, h / 2, h / 2, fill)
        }
    }

    companion object {
        /** Here, not a property: init's render() reads it before properties declared after init are set. */
        private val HAPTICS = listOf(
            "off" to "no touch from the motor",
            "light" to "soft ticks",
            "medium" to "a little firmer",
            "strong" to "firm ticks and clicks",
        )
        const val FAST_MS = 70L

        fun timeoutWords(s: Int): String = when {
            s < 60 -> "$s seconds"
            s == 60 -> "1 minute"
            else -> "${s / 60} minutes"
        }
    }
}

/**
 * An agent's space › What it can change (ADR-0045): for its look and home, the R1's settings, and apps' settings and
 * permissions, whether it **asks you** first (an approval card), is **allowed** without asking, or **never** may. A
 * click moves an area along. Granting an app a permission asks every time, whatever this says.
 */
class AgentCanScreen(shell: Shell, private val agentKey: String) : ListPage(shell, "What it can change", Theme.colorOf(shell.agent(agentKey)?.color)) {
    override val agentId get() = agentKey

    init {
        render()
    }

    override fun onAgents() = render()

    private fun render() {
        val keep = list.focus
        val a = shell.agent(agentKey) ?: return
        val can = a.raw.optJSONObject("can") ?: JSONObject()
        list.rebuild {
            note("When ${a.name} wants to change something on your R1:", size = 14f)
            val areas = if (a.computer != null) AREAS + PC_AREAS else AREAS
            for ((area, label, sub) in areas) {
                if (area == PC_AREAS[0].first) section("on ${a.computer}")
                // Changing files in its own folders is allowed until you say otherwise (crate::approvals).
                val level = can.optString(area).ifEmpty { if (area == "files") "allow" else "ask" }
                item(ICON[level], label, if (level == "allow") accent else null, sub = sub, trailing = WORDS[level]) {
                    val next = LEVELS[(LEVELS.indexOf(level) + 1) % LEVELS.size]
                    R1d.put("/api/agents/${R1d.enc(agentKey)}/can", JSONObject().put("area", area).put("level", next), onError = { shell.flash("couldn't change it") }) { r ->
                        r.optJSONObject("can")?.let { a.raw.put("can", it) }
                        render()
                    }
                }
            }
            note("Asking shows a card on the R1 for you to allow or not; the agent waits up to two minutes. Letting an app do " +
                "something new always asks.", size = 13f)
        }
        if (keep >= 0) list.setFocus(keep.coerceAtMost(list.size - 1), animate = false)
    }

    private companion object {
        val LEVELS = listOf("ask", "allow", "never")
        val WORDS = mapOf("ask" to "asks you", "allow" to "allowed", "never" to "never")
        val ICON = mapOf("ask" to "question", "allow" to "check-circle", "never" to "x-circle")
        val AREAS = listOf(
            Triple("look", "look and home", "colours, widgets, home"),
            Triple("device", "the R1's settings", "brightness, sleep, volume"),
            Triple("apps", "apps", "settings and permissions"),
            // The app window (ADR-0061): its first tap or typing in a task asks; each app and every final step always ask.
            Triple("android", "android apps", "tapping and typing in them"),
        )
        /** An agent on a computer: what it does there. */
        val PC_AREAS = listOf(
            Triple("files", "changing files", "in its folders"),
            Triple("commands", "running commands", "programs and scripts"),
            Triple("outside", "anything outside", "other files, opening apps"),
            // Your own Chrome (ADR-0053): looking runs; clicking and typing asks once per task.
            Triple("browser", "using your browser", "clicking and typing on pages"),
        )
    }
}

/**
 * An agent's space › Where it works (docs/specs/companion.md §7.2): on the R1, in its own space, or on a computer this
 * R1 is linked to, whose companion runs its turns. On a computer, how its new chats ask (each keeps its own); its
 * folders are chosen in the companion, on that computer.
 */
class AgentWhereScreen(shell: Shell, private val agentKey: String) : ListPage(shell, "Where it works", Theme.colorOf(shell.agent(agentKey)?.color)) {
    override val agentId get() = agentKey
    private var computers: List<JSONObject>? = null
    private var msg: String? = null

    init {
        render()
        R1d.get("/api/link", onError = { computers = emptyList(); render() }) { r ->
            computers = r.optJSONArray("computers")?.let { a -> (0 until a.length()).mapNotNull { a.optJSONObject(it) } }.orEmpty()
            render()
        }
    }

    override fun onAgents() = render()

    private fun render() {
        val keep = list.focus
        val a = shell.agent(agentKey) ?: return
        val now = a.raw.optString("works_on").ifEmpty { "r1" }
        list.rebuild {
            item(if (now == "r1") "check-circle" else "circle", "the R1", if (now == "r1") accent else null, sub = "in its own space here") { set("works_on", "r1") }
            val cs = computers
            if (cs == null) note("…") else for (c in cs) {
                val key = "computer:${c.optString("id")}"
                item(if (now == key) "check-circle" else "circle", c.optString("name"), if (now == key) accent else null,
                    sub = if (c.optBoolean("connected")) "linked now" else "not linked now") { set("works_on", key) }
            }
            if (cs != null && cs.isEmpty()) note("Link a computer first: Settings › Computers.", size = 13f)
            if (now != "r1") {
                section("new chats")
                val mode = a.raw.optString("mode").ifEmpty { "manual" }
                for ((m, label, sub) in MODES) {
                    item(if (mode == m) "check-circle" else "circle", label, if (mode == m) accent else null, sub = sub) { set("mode", m) }
                }
                note("Its folders, and what it may do there, are set in r1os on that computer (and in What it can change).", size = 13f)
            }
            msg?.let { note(it) }
        }
        if (keep >= 0) list.setFocus(keep.coerceAtMost(list.size - 1), animate = false)
    }

    private fun set(key: String, value: String) {
        val a = shell.agent(agentKey) ?: return
        R1d.put("/api/agents/${R1d.enc(agentKey)}", JSONObject(a.raw.toString()).put(key, value), onError = { msg = it; render() }) { r ->
            r.optJSONObject("agent")?.let { a.raw.put(key, it.opt(key)) }
            msg = null
            render()
        }
    }

    private companion object {
        val MODES = listOf(
            Triple("manual", "asks", "what it isn't allowed asks you here"),
            Triple("acceptEdits", "edits freely", "changes files in its folders without asking"),
            Triple("auto", "auto", "Claude decides what's safe"),
            Triple("plan", "plans only", "plans and explains, changes nothing"),
        )
    }
}

