package dev.r1os.host.shell

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextClock
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject
import kotlin.math.cos
import kotlin.math.sin

// Settings (launcher › Settings; ported from shell/src/settings.tsx) and the built-in tools
// (launcher › Tools; shell/src/apps.tsx). Each agent's own look, brain and memory are in its space.

fun settingsPage(shell: Shell, page: String): Screen? = when {
    // The look (ADR-0043): a colour being picked, a soft gradient's palettes.
    page.startsWith("look-color-") -> ColorPickScreen(shell, page.removePrefix("look-color-"))
    page.startsWith("look-palette-") -> PaletteScreen(shell, page.removePrefix("look-palette-"))
    else -> namedSettingsPage(shell, page)
}

private fun namedSettingsPage(shell: Shell, page: String): Screen? = when (page) {
    "you" -> MemoryScreen(shell, null)
    "agents" -> AgentsTalkScreen(shell)
    // Home's stops, in order, and what's pinned (T-109).
    "home" -> HomeStopsScreen(shell)
    "accounts" -> AccountsScreen(shell)
    "wifi" -> PanelPage(shell, "Wi-Fi", Theme.accent, WifiPanel(shell, "Done") { shell.back() })
    "about" -> AboutScreen(shell)
    "builds" -> BuildsScreen(shell)
    "time" -> DateTimeScreen(shell)
    // Brightness, sleep, volume and sounds (ADR-0045).
    "display" -> DisplayScreen(shell)
    // The wheel and the button, and turning it off (R7).
    "controls" -> ControlsScreen(shell)
    "bluetooth" -> BluetoothScreen(shell)
    // The apps your agents reach on your accounts (R8, ADR-0052).
    "connections" -> ConnectionsScreen(shell)
    "power" -> PowerScreen(shell)
    // The PIN lock (ADR-0016).
    "lock" -> LockScreen(shell)
    // Linked computers (ADR-0048).
    "computers" -> ComputersScreen(shell)
    // The look (ADR-0043).
    "look" -> LookScreen(shell)
    "look-colors" -> ColorsScreen(shell)
    "look-font" -> FontScreen(shell)
    "look-glance" -> GlanceLookScreen(shell)
    "look-clock" -> ClockFaceScreen(shell)
    "look-bg-glance" -> BackgroundScreen(shell, "glance")
    "look-bg-home" -> BackgroundScreen(shell, "home")
    "look-photo-glance" -> BackgroundPhotoScreen(shell, "glance")
    "look-photo-home" -> BackgroundPhotoScreen(shell, "home")
    else -> null
}

fun toolPage(shell: Shell, id: String, t: Target? = null): Screen? = when (id) {
    "alarms" -> AlarmsScreen(shell)
    "reminders" -> RemindersScreen(shell)
    "timer" -> TimerScreen(shell)
    "clock" -> ClockTool(shell)
    "camera" -> CameraScreen(shell)
    "photos" -> PhotosScreen(shell)
    "recorder" -> RecorderScreen(shell)
    "journal" -> JournalScreen(shell)
    // On the languages asked for, if any ("open the translator for English to Portuguese").
    "translator" -> TranslatorScreen(shell, t?.str("mine"), t?.str("theirs"))
    "whats_up" -> WhatsUpScreen(shell)
    "routines" -> RoutinesScreen(shell)
    "notifications" -> NotificationsScreen(shell)
    "inputs" -> InputsTool(shell)
    "system" -> SystemTool(shell)
    else -> null
}

/** A key and its value, on one line (`.kv`). */
private fun FocusList.kv(k: String, v: String) {
    place(LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        setPadding(context.dpi(6f), context.dpi(8f), context.dpi(6f), context.dpi(8f))
        addView(context.text(16f, 500, Theme.MUTED).apply { text = k }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        addView(context.text(16f, 600).apply { text = v; gravity = Gravity.END })
    })
}

/** The sign-ins every agent shares. Which one an agent talks with is set in its space. */
class AccountsScreen(shell: Shell) : ListPage(shell, "Accounts") {
    private var acc: JSONObject? = null
    /** Signing out of one asks twice: it's every agent's sign-in. */
    private val confirm = Confirm { render() }

    init {
        list.gap(10f)
        load()
        render()
    }

    private fun load(): Unit = R1d.get("/api/accounts") { acc = it.optJSONObject("accounts"); render() }

    override fun onShow() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "account" && (ev.str("state") == "signed_in" || ev.str("state") == "signed_out")) load()
    }

    private fun render(): Unit = list.rebuild {
        for ((b, label) in listOf("claude" to "Claude", "codex" to "Codex")) {
            val on = acc?.optBoolean(b) == true
            place(context.card(
                context.text(20f, 750).apply { text = label },
                context.text(15f, 500, Theme.MUTED).apply { text = if (acc == null) "…" else if (on) "Connected · every agent can use it" else "Not connected" },
                ok = on,
            ))
            if (on) {
                val armed = confirm.armed == b
                button(if (armed) "Click again to sign out of $label" else "Sign out of $label", primary = false, small = true) {
                    confirm.tap(b) { R1d.post("/api/accounts/$b/logout") { load() } }
                }.tag = "out-$b"
            }
        }
        button("Connect an account") {
            val panel = BrainPanel(shell, shell.focusAgent().raw, "Done") { shell.back() }
            shell.push(PanelPage(shell, "Connect an account", Theme.accent, panel))
        }.tag = "connect"
        button("Done", primary = false) { shell.back() }.tag = "done"
    }

    override fun onDestroy() = confirm.cancel()
}

/** What's running: r1d's version, the battery, free memory. And setup again. */
class AboutScreen(shell: Shell) : ListPage(shell, "About") {
    private var state: JSONObject? = R1d.state
    /** r1d's `/api/boot`: verbose boot (T-101). */
    private var boot: JSONObject? = null
    private var msg: String? = null

    init {
        render()
        R1d.get("/api/state") { state = it; render() }
        R1d.get("/api/boot") { boot = it; render() }
    }

    private fun render(): Unit = list.rebuild {
        val s = state
        val sys = s?.optJSONObject("system")
        kv("r1os", s?.let { "r1d v${it.optString("version")}" } ?: "…")
        kv("Battery", sys?.takeIf { !it.isNull("battery_pct") }?.let { "${it.optInt("battery_pct")}%" + if (it.optBoolean("charging")) " (charging)" else "" } ?: "…")
        kv("Memory free", sys?.takeIf { !it.isNull("mem_available_mb") }?.let { "${it.optInt("mem_available_mb")} MB" } ?: "…")
        kv("Mode", if (s?.optBoolean("sim") == true) "simulator" else "R1")
        boot?.let { b ->
            val on = b.optBoolean("verbose")
            item("terminal-window", "verbose boot", if (on) accent else null, sub = "the kernel's log on screen as it starts", trailing = if (on) "on" else "off") {
                R1d.put("/api/boot", JSONObject().put("verbose", !on), onError = { msg = it; render() }) { boot = it; msg = null; render() }
            }
        }
        msg?.let { note(it) }
        item("heart", "credits", sub = "what r1os is built on, and their licences") { shell.push(CreditsScreen(shell)) }.tag = "credits"
        button("Run setup again", primary = false) {
            R1d.put("/api/onboarding", JSONObject().put("step", "hello")) { shell.restartSetup() }
        }
    }
}

/**
 * Settings › About › Credits (T-178): what r1os on the R1 is built on, and each part's licence, as NOTICE.md in r1os's
 * source says (keep the two in step), plus what the agent box downloads. One row each, so the wheel reads them one at a
 * time.
 */
class CreditsScreen(shell: Shell) : ListPage(shell, "Credits") {
    init {
        list.rebuild {
            note("r1os is open source under the MIT licence. It's built on the work of others, each under its own licence: " +
                "the full texts are in NOTICE.md in r1os's source.", size = 13f)
            for ((group, rows) in CREDITS) {
                section(group)
                for ((name, licence) in rows) item(null, name, sub = licence) {}
            }
            button("Done", primary = false) { shell.back() }.tag = "done"
        }
    }

    private companion object {
        val CREDITS = listOf(
            "on the r1" to listOf(
                "kroko" to "banafo · cc-by-sa · speech to text",
                "all-minilm-l6-v2" to "sentence-transformers · apache-2.0 · system one",
                "whisper base" to "openai · mit · the translator's ears",
                "opus-mt" to "helsinki-nlp · cc-by-4.0 · the translator",
                "piper voices" to "each under its dataset's terms",
                "espeak-ng" to "its data · gpl-3.0 or later",
                "sherpa-onnx" to "k2-fsa · apache-2.0",
                "onnx runtime" to "microsoft · mit",
            ),
            "the system" to listOf(
                "android" to "the android open source project · apache-2.0 and others",
                "trebledroid" to "phhusson and contributors",
                "aurora store" to "aurora oss · gpl-3.0 or later",
                "rabbit's kernel and vendor" to "rabbit inc. · the kernel under gpl-2.0",
            ),
            "the agent box" to listOf(
                "alpine linux" to "its packages, each under its own licence",
                "claude code" to "anthropic · its own terms",
                "codex" to "openai · apache-2.0",
            ),
            "the look" to listOf(
                "rubik" to "sil open font licence 1.1",
                "phosphor icons" to "mit",
            ),
            "with thanks" to listOf(
                "r1ptt" to "julian dice · mit",
                "rabbit_r1_boot_notes" to "david buchanan",
            ),
        )
    }
}

/**
 * Settings › Agents: whether your agents may wake each other (ADR-0012). Paused, their messages still arrive
 * (a badge, Messages in the agent's space) but wait until you talk to that agent.
 */
class AgentsTalkScreen(shell: Shell) : ListPage(shell, "Agents") {
    private var paused: Boolean? = null

    init {
        render()
        load()
    }

    private fun load(): Unit = R1d.get("/api/agents") { paused = it.optBoolean("paused"); render() }

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "agents_changed") load()
    }

    private fun render(): Unit = list.rebuild {
        val p = paused ?: run { note("…"); return@rebuild }
        item(
            "chats-circle", "talk to each other", if (!p) accent else null,
            sub = if (p) "paused: their messages wait until you talk to them" else "they can ask each other for help",
            trailing = if (p) "off" else "on",
        ) {
            R1d.put("/api/agents/paused", JSONObject().put("paused", !p)) { paused = it.optBoolean("paused"); render() }
        }
        note("An agent can ask another when something is that one's area. Their messages are in each agent's space. An agent " +
            "is woken by the others at most 10 times an hour, a chain stops after 4 questions, and a turn another agent " +
            "started can't delete or change your apps.", size = 13f)
    }
}

/**
 * Settings › App builds: which Claude model builds and changes your apps, from chats and from New app. An
 * agent can still pick another for one build, when you ask it to.
 */
class BuildsScreen(shell: Shell) : ListPage(shell, "App builds") {
    private var model: String? = null

    init {
        render()
        R1d.get("/api/builds/model") { model = it.optString("model"); render() }
    }

    private fun render(): Unit = list.rebuild {
        section("model")
        for ((id, sub) in MODELS) {
            val on = id == model
            item(if (on) "check-circle" else "circle", id, if (on) accent else null, sub = sub) {
                R1d.put("/api/builds/model", JSONObject().put("model", id)) { model = it.optString("model"); render() }
            }
        }
    }

    private companion object {
        val MODELS = listOf(
            "haiku" to "fastest, for small apps", "sonnet" to "the usual: quick and careful", "opus" to "slowest, for the hard ones",
            "codex" to "chatgpt's, through your codex sign-in",
        )
    }
}

/**
 * Settings › Date & time: the R1's time zone. It follows the R1's network (ADR-0030): looked up each time
 * it connects. Picking one by voice ("set my time zone to London") stops that; a click here starts it again.
 * The date and time themselves come from the network (Android's network time).
 */
class DateTimeScreen(shell: Shell) : ListPage(shell, "Date & time") {
    private var time: JSONObject? = null

    init {
        render()
        load()
    }

    private fun load(): Unit = R1d.get("/api/time") { time = it; render() }

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "alarms_changed") load()
    }

    private fun render(): Unit = list.rebuild {
        val t = time ?: run { note("…"); return@rebuild }
        val auto = t.optBoolean("automatic")
        kv("Time zone", t.optString("zone").replace('_', ' '))
        kv("Now", whenText(context, t.optLong("now")))
        item("globe-hemisphere-west", "set automatically", if (auto) accent else null, sub = if (auto) "from where the r1 is connected" else "picked by you", trailing = if (auto) "on" else "off") {
            R1d.post("/api/time", JSONObject().put("automatic", !auto)) { time = it; render() }
        }
        val h24 = android.text.format.DateFormat.is24HourFormat(context)
        item("clock", "24-hour clock", if (h24) accent else null, sub = if (h24) "17:30" else "5:30 pm", trailing = if (h24) "on" else "off") {
            dev.r1os.host.DeviceSettings.apply(context, JSONObject().put("clock_24h", !h24))
            // Android writes it at once; the row reads it again a moment later.
            list.postDelayed({ render() }, 150)
        }
    }
}

/** A live log of r1d's events: the tool for tuning gestures and the wheel guard (Q12). */
class InputsTool(shell: Shell) : PageScreen(shell, "Inputs") {
    private val head = context.text(13f, 500, Theme.MUTED)
    private val log = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val empty = context.text(15f, 500, Theme.MUTED).apply { text = "Spin the wheel or press the button…" }

    init {
        val s = R1d.state
        val dev = s?.optJSONObject("devices")
        val st = s?.optJSONObject("stats")
        head.text = "wheel ${dev?.optString("wheel")?.takeIf { it.isNotEmpty() && it != "null" } ?: "—"} · button ${dev?.optString("button")?.takeIf { it.isNotEmpty() && it != "null" } ?: "—"}" +
            (st?.let { " · since start: ${it.optInt("wheel")} ticks, ${it.optInt("wheel_suppressed")} suppressed" } ?: "")
        val col = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(context.dpi(14f), 0, context.dpi(14f), context.dpi(12f))
            addView(head)
            addView(ScrollView(context).apply {
                isVerticalScrollBarEnabled = false
                addView(log)
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f).apply { topMargin = context.dpi(8f) })
        }
        log.addView(empty)
        body.addView(col, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }

    /** Observe only: nothing is consumed (double tap still goes back). */
    override fun onInput(ev: Ev): Boolean = false

    override fun onAnyEvent(ev: Ev) {
        if (shell.top() !== this || ev.type == "voice_level") return
        if (empty.parent != null) log.removeView(empty)
        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, context.dpi(3f), 0, context.dpi(3f))
            addView(context.text(13f, 500, Theme.MUTED).apply { text = String.format("%.2f", ev.num("t_ms") / 1000); fontFeatureSettings = "tnum"; minWidth = context.dpi(64f) })
            addView(context.text(15f, 500).apply { text = describe(ev) }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            if (ev.str("source") == "sim") addView(context.text(10f, 600, Theme.MUTED).apply { text = "SIM" })
        }
        log.addView(row, 0)
        while (log.childCount > 40) log.removeViewAt(log.childCount - 1)
    }

    private fun describe(e: Ev): String = when (e.type) {
        "wheel" -> if (e.int("delta") > 0) "wheel ↓" else "wheel ↑"
        "wheel_suppressed" -> "wheel ${if (e.int("delta") > 0) "↓" else "↑"} (suppressed)"
        "button" -> if (e.bool("down")) "button down" else "button up"
        "gesture" -> (e.str("kind") ?: "").replace("_", " ") + (if (e.json.has("held_ms")) " · ${e.int("held_ms")} ms" else "")
        "motor" -> "motor → ${e.int("deg")}°"
        "app_changed" -> "${e.str("app")}: changed"
        "app_installed" -> "${e.str("app")} installed"
        "app_removed" -> "${e.str("app")} removed"
        "build" -> "build ${e.int("id")}: ${e.str("stage")}" + (e.str("section")?.let { " · $it" } ?: "")
        "account" -> "${e.str("brain")}: ${e.str("state")?.replace("_", " ")}"
        "chat" -> "chat ${e.str("kind")}"
        "agent_state" -> "agent ${e.str("state")}"
        "agents_changed" -> "agents changed"
        "unread" -> "${e.int("count")} unread"
        "open" -> "open ${e.str("app")}"
        "voice" -> "voice ${e.str("state")}" + (e.str("text")?.takeIf { it.isNotEmpty() }?.let { ": $it" } ?: "")
        else -> e.type
    }
}


/** The time, big, with seconds and the date. */
class ClockTool(shell: Shell) : PageScreen(shell, "Clock") {
    init {
        val col = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setPadding(0, 0, 0, context.dpi(40f))
        }
        fun clock(f12: String, f24: String, size: Float, weight: Int, color: Int) = TextClock(context).apply {
            format12Hour = f12
            format24Hour = f24
            setTextSize(android.util.TypedValue.COMPLEX_UNIT_DIP, size)
            typeface = Theme.font(weight)
            setTextColor(color)
            fontFeatureSettings = "tnum"
            gravity = Gravity.CENTER
            includeFontPadding = false
            transformationMethod = dev.r1os.host.ui.Lowercase
        }
        col.addView(clock("h:mm", "HH:mm", 76f, 700, Theme.FG))
        col.addView(clock("ss", "ss", 26f, 600, Theme.MUTED), LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(4f) })
        col.addView(clock("EEEE, MMMM d", "EEEE, MMMM d", 16f, 500, Theme.MUTED), LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(14f) })
        body.addView(col, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }

    override fun onInput(ev: Ev): Boolean {
        if (ev.type == "wheel" && ev.int("delta") < 0) { overscroll(); return true }
        return ev.type == "wheel"
    }
}


/** r1d's state, refreshed every 2 s while it's on screen. */
class SystemTool(shell: Shell) : ListPage(shell, "System") {
    private var state: JSONObject? = null
    private val poll = object : Runnable {
        override fun run() {
            R1d.get("/api/state") { state = it; render() }
            R1d.main.postDelayed(this, 2000)
        }
    }

    init { render() }

    override fun onShow() {
        R1d.main.removeCallbacks(poll)
        poll.run()
    }

    override fun onHide() {
        super.onHide()
        R1d.main.removeCallbacks(poll)
    }

    override fun onDestroy() = R1d.main.removeCallbacks(poll)

    private fun render(): Unit = list.rebuild {
        val s = state
        if (s == null) { note("Loading…"); return@rebuild }
        val sys = s.optJSONObject("system")
        val motor = s.optJSONObject("motor")
        kv("Battery", sys?.takeIf { !it.isNull("battery_pct") }?.let { "${it.optInt("battery_pct")}%" + if (it.optBoolean("charging")) " (charging)" else "" } ?: "—")
        kv("Memory free", sys?.takeIf { !it.isNull("mem_available_mb") }?.let { "${it.optInt("mem_available_mb")} / ${it.optInt("mem_total_mb")} MB" } ?: "—")
        kv("Camera", motor?.takeIf { !it.isNull("deg") }?.let { "${it.optInt("deg")}°" + if (it.optBoolean("sim")) " (sim)" else "" } ?: "—")
        val up = s.optLong("uptime_s")
        kv("r1d", "v${s.optString("version")} · up " + if (up >= 3600) "${up / 3600}h ${up % 3600 / 60}m" else "${up / 60}m ${up % 60}s")
        kv("Mode", if (s.optBoolean("sim")) "simulator" else "hardware")
        kv("Clients", s.optInt("clients").toString())
    }
}
