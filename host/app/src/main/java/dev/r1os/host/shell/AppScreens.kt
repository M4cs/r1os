package dev.r1os.host.shell

import android.graphics.Color
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject

// Managing apps and making them (docs/specs/experience.md §2.6, docs/specs/apps.md §4; ported from
// shell/src/manage.tsx and create.tsx). Changes are made by voice: on an app's page, hold the button
// and say what to change.

private fun accentOf(app: JSONObject?) = Theme.COLORS[app?.optString("accent").orEmpty()] ?: Color.parseColor("#9aa4b2")

private fun JSONObject.strings(key: String): List<String> = optJSONArray(key)?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()

private fun JSONObject.list(key: String): List<JSONObject> {
    val a = optJSONArray(key) ?: JSONArray()
    return (0 until a.length()).mapNotNull { a.optJSONObject(it) }
}

/** "Thinking out loud" rows: the accent's dot and a line of muted text (`.create-talk`). */
private fun FocusList.talk(s: String, accent: Int) {
    val row = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(context.dpi(6f), context.dpi(8f), context.dpi(6f), context.dpi(8f))
        addView(View(context).apply { background = roundRect(accent, context.dpi(4f).toFloat()) }, LinearLayout.LayoutParams(context.dpi(8f), context.dpi(8f)))
        addView(context.text(17f, 500).apply { text = s; setPadding(context.dpi(10f), 0, 0, 0) })
    }
    place(row)
}

/** All apps, then the trash. Click an app for its page; click a trashed one to bring it back. */
class ManageAppsScreen(shell: Shell) : ListPage(shell, "Manage apps") {
    private var apps: List<JSONObject> = emptyList()
    private var trash: List<JSONObject> = emptyList()
    private var keepDays = 7
    private var msg: String? = null
    private var loaded = false

    init {
        load()
        render()
    }

    private fun load() {
        fetch("/api/apps", ::render) { apps = it.list("apps"); loaded = true }
        fetch("/api/trash", ::render) { trash = it.list("apps"); keepDays = it.optInt("keep_days", 7) }
    }

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "app_installed" || ev.type == "app_removed") load()
    }

    private fun render(): Unit = list.rebuild {
        item("scan", "Add from a code", accent, sub = "another r1's app, shared by QR") { shell.push(AppCodeScreen(shell)) }
        if (loaded && apps.isEmpty()) note("No apps yet.")
        for (a in apps) {
            val id = a.optString("id")
            item(a.optString("icon", "app-window"), a.optString("name", id), accentOf(a), trailing = "›") { shell.open(Target.of("app-info", "id" to id)) }
        }
        if (trash.isNotEmpty()) section("Trash · kept $keepDays days")
        for (t in trash) {
            val name = t.optString("name")
            item("arrow-counter-clockwise", name, trailing = ago(t.optLong("removed_at"), long = true)) {
                R1d.post("/api/apps/${R1d.enc(t.optString("id"))}/restore", onError = { msg = it; render() }) { msg = "$name is back."; render() }
            }
        }
        msg?.let { note(it) }
    }
}

/**
 * One app: what it is, change it by voice, open, its settings and permissions, undo, erase its data,
 * remove, and its history.
 *
 * Settings change with the button and wheel: a click flips a switch or moves to the next option; on a
 * number, it starts turning (the wheel changes it, a click keeps it, a double click puts it back); text
 * opens a field.
 */
class AppInfoScreen(shell: Shell, private val appId: String, startedBuild: Long? = null) : ListPage(shell, "app") {
    private var app: JSONObject? = null
    private var versions: List<JSONObject> = emptyList()
    /** Its settings' definitions (compiled) and their values now. */
    private var settingDefs: JSONObject? = null
    private var settingValues = JSONObject()
    /** A number setting being turned with the wheel: its key and its value so far. */
    private var turning: Pair<String, Double>? = null
    /** What it may do beyond its own data, with your decisions (ADR-0023). */
    private var permissions: List<JSONObject> = emptyList()
    /** What went wrong on its page since its last version: a fix repairs it. */
    private var problems: List<JSONObject> = emptyList()
    /** A change being built: its stage and section. */
    private var busy: Pair<String, String?>? = startedBuild?.let { "starting" to null }
    private var build: Long? = startedBuild
    private var msg: String? = null
    private val confirm: Confirm = Confirm { render() }
    /** The apps pinned to home (ADR-0044), to show and change whether this one is. */
    private var pins: List<String>? = null
    /** The apps whose cards are on the glance screen, in order (ADR-0042), to show and change whether this one's is. */
    private var glance: List<String>? = null

    init {
        load()
        render()
    }

    private fun load() {
        fetch("/api/apps", ::render) { r -> app = r.list("apps").firstOrNull { it.optString("id") == appId }; app?.let { header.setTitle(it.optString("name")) } }
        fetch("/api/apps/${R1d.enc(appId)}/history", ::render, onError = { versions = emptyList(); soon(::render) }) { versions = it.list("versions") }
        fetch("/api/apps/${R1d.enc(appId)}", ::render) { settingDefs = it.optJSONObject("app")?.optJSONObject("settings") }
        loadSettings()
        loadPermissions()
        loadProblems()
        fetch("/api/home", ::render) { r -> pins = r.strings("pins") }
        fetch("/api/glance", ::render) { r -> glance = r.list("widgets").map { it.optString("app") } }
    }

    private fun pin(on: Boolean) {
        val now = pins ?: return
        val next = if (on) now + appId else now - appId
        R1d.put("/api/home", JSONObject().put("pins", org.json.JSONArray(next)), onError = { msg = it; render() }) { r ->
            pins = r.strings("pins")
            render()
        }
    }

    /** On, its card goes last; the companion orders them. */
    private fun onGlance(on: Boolean) {
        val now = glance ?: return
        val next = if (on) now + appId else now - appId
        R1d.put("/api/glance", JSONObject().put("apps", org.json.JSONArray(next)), onError = { msg = it; render() }) { r ->
            glance = r.list("widgets").map { it.optString("app") }
            render()
        }
    }

    private fun loadProblems() =
        fetch("/api/apps/${R1d.enc(appId)}/errors", ::render, onError = { problems = emptyList(); soon(::render) }) { problems = it.list("errors") }

    /** An edit build asked to repair what went wrong (r1d writes the request from the problems). */
    private fun fix() {
        msg = null
        busy = "starting" to null
        render()
        R1d.post("/api/apps/${R1d.enc(appId)}/fix", onError = { busy = null; msg = it; render() }) { build = it.optLong("build") }
    }

    private fun loadSettings() = fetch("/api/apps/${R1d.enc(appId)}/settings", ::render) { settingValues = it }

    private fun saveSetting(key: String, value: Any) {
        val body = JSONObject().put("values", JSONObject().put(key, value))
        R1d.post("/api/apps/${R1d.enc(appId)}/settings", body, onError = { msg = it; render() }) { settingValues = it; render() }
    }

    /** A click on a setting: flip it, move on to the next option, start turning it, or open a field. */
    private fun onSetting(key: String, def: JSONObject, label: String, now: Any?) {
        when (val type = def.optString("type")) {
            "bool" -> saveSetting(key, !(now == true || now == 1))
            "enum" -> {
                val options = def.optJSONArray("options") ?: return
                val all = (0 until options.length()).map { options.optString(it) }
                val next = all.getOrNull((all.indexOf(now?.toString()) + 1) % all.size) ?: return
                saveSetting(key, next)
            }
            "int", "real", "duration" -> {
                turning = key to ((now as? Number)?.toDouble() ?: now?.toString()?.toDoubleOrNull() ?: def.optDouble("default", 0.0))
                Sounds.play(Sounds.S.OPEN)
                render()
            }
            else -> shell.push(SettingTextScreen(shell, appId, key, label, now?.toString().orEmpty(), type))
        }
    }

    /** One wheel tick on a setting being turned: up is more. */
    private fun turn(delta: Int) {
        val (key, value) = turning ?: return
        val def = settingDefs?.optJSONObject(key) ?: return
        val type = def.optString("type")
        val step = when (type) {
            "real" -> if (kotlin.math.abs(value) < 10) 0.1 else 1.0
            "duration" -> when { value < 60 -> 5.0; value < 3600 -> 60.0; else -> 300.0 }
            else -> 1.0
        }
        var next = value - delta * step
        if (def.has("min")) next = next.coerceAtLeast(def.optDouble("min"))
        if (def.has("max")) next = next.coerceAtMost(def.optDouble("max"))
        if (type != "real") next = kotlin.math.round(next) else next = kotlin.math.round(next * 10) / 10
        if (next == value) { Sounds.play(Sounds.S.BUMP); return }
        Sounds.play(Sounds.S.TICK)
        turning = key to next
        render()
    }

    private fun loadPermissions() =
        fetch("/api/apps/${R1d.enc(appId)}/permissions", ::render, onError = { permissions = emptyList(); soon(::render) }) { permissions = it.list("permissions") }

    /** A click flips it: allowed ↔ not allowed (one that hasn't been asked yet becomes allowed). */
    private fun toggle(p: JSONObject) {
        val allow = p.optString("state") != "granted"
        val body = JSONObject().put("decisions", JSONObject().put(p.optString("id"), allow)).put("via", "settings")
        R1d.post("/api/apps/${R1d.enc(appId)}/permissions", body, onError = { msg = it; render() }) { permissions = it.list("permissions"); render() }
    }

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "app_permissions" && ev.str("app") == appId) loadPermissions()
        if (ev.type == "app_errors" && ev.str("app") == appId) loadProblems()
        if (ev.type == "app_changed" && ev.str("app") == appId && turning == null) {
            val tables = ev.json.optJSONArray("tables")
            if (tables != null && (0 until tables.length()).any { tables.optString(it) == "settings" }) loadSettings()
        }
        if (ev.type == "app_installed" && ev.str("app") == appId) load()
        if (ev.type == "build" && build != null && ev.json.optLong("id") == build) {
            when (ev.str("stage")) {
                "done" -> {
                    busy = null
                    val said = ev.str("said") ?: "Changed."
                    val deleted = ev.json.optJSONArray("deleted")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
                    msg = if (deleted.isNotEmpty()) "$said Deleted: ${deleted.joinToString(", ")}." else said
                }
                "failed" -> { busy = null; msg = "That change didn't work: ${ev.str("error") ?: "unknown error"}" }
                else -> busy = (ev.str("stage") ?: "writing") to ev.str("section")
            }
            render()
        }
    }

    /** Held-button words change this app. While a number setting turns, the wheel and clicks are its. */
    override fun extra(ev: Ev): Boolean {
        turning?.let { (key, value) ->
            when {
                ev.type == "wheel" -> { turn(ev.int("delta")); return true }
                ev.isTap -> {
                    turning = null
                    val type = settingDefs?.optJSONObject(key)?.optString("type")
                    saveSetting(key, if (type == "real") value else value.toLong())
                    return true
                }
                ev.isDoubleTap -> { turning = null; Sounds.play(Sounds.S.BACK); render(); return true }
                else -> {}
            }
        }
        if (ev.type != "voice" || ev.str("state") != "final") return false
        val text = ev.str("text")?.trim().orEmpty()
        if (text.isEmpty()) return false
        if (busy == null) change(text)
        return true
    }

    private fun change(request: String) {
        msg = null
        busy = "starting" to null
        render()
        R1d.post("/api/apps/${R1d.enc(appId)}/edit", JSONObject().put("request", request), onError = { busy = null; msg = it; render() }) { build = it.optLong("build") }
    }

    private fun undo() = R1d.post("/api/apps/${R1d.enc(appId)}/undo", onError = { msg = it; render() }) { r ->
        val u = r.optJSONObject("undone")
        val restored = r.optJSONArray("restored")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
        msg = "Undid: ${u?.optString("said")?.ifEmpty { null } ?: u?.optString("request").orEmpty()}" + if (restored.isNotEmpty()) " · brought back ${restored.joinToString(", ")}" else ""
        render()
    }

    private fun render(): Unit = list.rebuild {
        val a = app
        if (a == null) { note("…"); return@rebuild }
        val accent = accentOf(a)
        // The head: its icon, name and what it's for.
        val head = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(context.dpi(4f), 0, 0, context.dpi(4f))
            addView(FrameLayout(context).apply {
                background = roundRect(Theme.withAlpha(accent, 0.2f), context.dpi(11f).toFloat())
                addView(android.widget.ImageView(context).apply { setImageDrawable(dev.r1os.host.ui.Icons.drawable(a.optString("icon", "app-window"), accent)) },
                    FrameLayout.LayoutParams(context.dpi(22f), context.dpi(22f), Gravity.CENTER))
            }, LinearLayout.LayoutParams(context.dpi(40f), context.dpi(40f)))
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(context.dpi(12f), 0, 0, 0)
                addView(context.text(19f, 700).apply { text = a.optString("name") })
                a.optString("description").takeIf { it.isNotEmpty() }?.let { d -> addView(context.text(13f, 500, Theme.MUTED).apply { text = d }) }
            }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        }
        place(head)
        // A change being built (you hold the button and say it): its progress, and nothing at rest.
        busy?.let { b ->
            talk(if (b.first == "repairing") "Fixing up the change…" else "Changing it${b.second?.let { " · $it" } ?: "…"}", accent)
        }
        msg?.let { note(it) }
        item("play", "Open", accent) { shell.open(Target.of("app", "id" to appId)) }
        // On home (ADR-0044): after the agents on the wheel.
        pins?.let { p ->
            val on = appId in p
            item("push-pin", "on home", if (on) accent else null, sub = "a stop after your agents", trailing = if (on) "on" else "off") { pin(!on) }
        }
        // On the glance screen (ADR-0042, T-050): only an app with a glance card has one to show.
        glance?.takeIf { a.optBoolean("glance") }?.let { g ->
            val on = appId in g
            item("cards", "on the glance screen", if (on) accent else null, sub = "its card where the r1 wakes", trailing = if (on) "on" else "off") { onGlance(!on) }
        }
        // Something went wrong when it ran: the first problem, and a fix.
        problems.lastOrNull()?.let { p ->
            val more = if (problems.size > 1) " · and ${problems.size - 1} more" else ""
            if (busy == null) item("wrench", "Fix it", Theme.BAD, sub = p.optString("message") + more) { fix() }
        }
        val defs = settingDefs
        if (defs != null && defs.length() > 0) {
            section("settings")
            for (key in defs.keys()) {
                val def = defs.optJSONObject(key) ?: continue
                val label = def.optString("label").ifEmpty { key.replace('_', ' ') }
                val now = if (settingValues.has(key)) settingValues.opt(key) else def.opt("default")
                val turned = turning?.takeIf { it.first == key }
                val shown = settingText(def.optString("type"), turned?.second ?: now)
                item(null, label, if (turned != null) accent else null, sub = if (turned != null) "turn to change · click to keep" else null,
                    trailing = if (turned != null) "‹ $shown ›" else shown) { if (turning == null) onSetting(key, def, label, now) }
            }
        }
        val changes = versions.filter { it.optString("kind") != "create" }
        changes.lastOrNull()?.let { last ->
            item("arrow-u-up-left", "undo: ${last.optString("said").ifEmpty { last.optString("request") }}") { undo() }
        }
        if (permissions.isNotEmpty()) section("permissions")
        for (p in permissions) {
            val state = p.optString("state")
            val used = p.optLong("used_at").takeIf { it > 0 }?.let { " · used ${ago(it, long = true)}" }.orEmpty()
            val sub = when (state) {
                "granted" -> "allowed$used"
                "denied" -> "not allowed"
                "ask" -> "asks when it opens"
                else -> "not on the r1 yet"
            } + p.optString("reason").takeIf { it.isNotBlank() }?.let { " · $it" }.orEmpty()
            val icon = when (p.optString("id")) { "net.http" -> "globe"; "notify" -> "bell"; "schedule" -> "clock"; else -> "lock" }
            item(icon, p.optString("label"), if (state == "granted") accent else null, sub = sub, trailing = when (state) { "granted" -> "on"; "unavailable" -> null; else -> "off" }) {
                if (state != "unavailable") toggle(p)
            }
        }
        item("qr-code", "Share by QR", sub = "another r1 adds it from the code; a phone gets its file") {
            shell.push(ShareScreen(shell, JSONObject().put("app", appId), app?.optString("name").orEmpty().ifEmpty { appId }))
        }
        val erasing = confirm.armed == "erase"
        item("eraser", if (erasing) "Click again to erase its data" else "Erase its data", danger = erasing) {
            confirm.tap("erase") {
                R1d.post("/api/apps/${R1d.enc(appId)}/erase", onError = { msg = it; render() }) { msg = "Erased. It starts over, empty."; loadSettings() }
            }
        }
        val armed = confirm.armed == "remove"
        item("trash", if (armed) "Click again to remove (kept 7 days)" else "Remove", danger = armed) {
            confirm.tap("remove") { R1d.delete("/api/apps/${R1d.enc(appId)}", onError = { msg = it; render() }) { shell.back() } }
        }
        if (versions.isNotEmpty()) section("History")
        for (v in versions.reversed()) {
            val what = if (v.optString("kind") == "create") "Created" + v.optString("request").let { if (it.isNotEmpty()) ": $it" else "" }
            else v.optString("said").ifEmpty { v.optString("request") }
            val deleted = v.optJSONArray("deleted")?.let { d -> (0 until d.length()).map { d.optString(it) } }.orEmpty()
            val row = LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(context.dpi(6f), context.dpi(4f), context.dpi(6f), context.dpi(4f))
                addView(context.text(12f, 500, Theme.MUTED).apply { text = ago(v.optLong("at"), long = true) })
                addView(context.text(15f, 500).apply { text = what })
                if (deleted.isNotEmpty()) addView(context.text(13f, 500, Theme.BAD).apply { text = "deleted ${deleted.joinToString(", ")}" })
            }
            place(row)
        }
    }

    override fun onDestroy() = confirm.cancel()
}

/** A setting's value as its row shows it. */
private fun settingText(type: String, v: Any?): String = when {
    v == null || v == JSONObject.NULL -> "–"
    type == "bool" -> if (v == true || v == 1) "on" else "off"
    type == "duration" -> {
        val s = (v as? Number)?.toLong() ?: v.toString().toLongOrNull() ?: 0
        when { s % 3600 == 0L && s > 0 -> "${s / 3600} h"; s % 60 == 0L && s > 0 -> "${s / 60} min"; else -> "$s s" }
    }
    type == "real" -> (v as? Number)?.toDouble()?.let { d -> if (d == kotlin.math.floor(d)) d.toLong().toString() else "%.1f".format(java.util.Locale.ROOT, d) } ?: v.toString()
    type == "int" -> (v as? Number)?.toLong()?.toString() ?: v.toString()
    else -> v.toString().ifEmpty { "–" }
}

/** One of an app's text settings (a town, a ticker): a field, where a click brings up the keyboard, and save. */
class SettingTextScreen(shell: Shell, private val appId: String, private val key: String, title: String, private var value: String, private val type: String) : ListPage(shell, title) {
    private var msg: String? = null
    private var input: android.widget.EditText? = null

    init { render() }

    private fun render(): Unit = list.rebuild {
        input = field(if (type == "time") "like 09:00" else "", value, maxLength = 200)
        button("save") { save() }
        msg?.let { note(it) }
    }

    private fun save() {
        value = input?.text?.toString().orEmpty()
        val body = JSONObject().put("values", JSONObject().put(key, value))
        R1d.post("/api/apps/${R1d.enc(appId)}/settings", body, onError = { msg = it; render() }) { shell.back() }
    }
}

/**
 * New app: describe it by holding the button, or pick an example, and the agent in the R1's box
 * builds it (r1d `/api/create`). The build's progress shows until it opens.
 */
class CreateScreen(shell: Shell) : ListPage(shell, "New app") {
    private var stage = "idle"
    private var brief = ""
    private var section: String? = null
    private var chars = 0
    private var error: String? = null
    private var problems: List<String> = emptyList()
    private var buildId: Long? = null
    private var doneText = ""

    private val progress = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        gravity = Gravity.CENTER
        setPadding(context.dpi(24f), 0, context.dpi(24f), context.dpi(30f))
        visibility = View.GONE
    }
    private val worker = CharacterView(context, context.dpi(110f))
    private val briefText = context.text(18f, 650).apply { gravity = Gravity.CENTER }
    private val stageText = context.text(16f, 500, Theme.MUTED).apply { gravity = Gravity.CENTER }

    init {
        progress.addView(worker, LinearLayout.LayoutParams(context.dpi(110f), context.dpi(110f)))
        progress.addView(briefText, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(16f) })
        progress.addView(stageText, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(10f) })
        body.addView(progress, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        render()
    }

    private val building get() = stage != "idle" && stage != "failed"

    override fun onInput(ev: Ev): Boolean {
        // While it builds, the wheel and clicks do nothing here (double tap still goes back).
        if (building) return ev.type == "wheel" || ev.isTap
        return super.onInput(ev)
    }

    override fun extra(ev: Ev): Boolean {
        if (ev.type == "voice" && ev.str("state") == "final") {
            val t = ev.str("text")?.trim().orEmpty()
            if (t.isEmpty()) return false
            start(t)
            return true
        }
        return false
    }

    override fun onAnyEvent(ev: Ev) {
        if (ev.type != "build" || buildId == null || ev.json.optLong("id") != buildId) return
        when (val s = ev.str("stage")) {
            "done" -> {
                stage = "done"
                val secs = ev.num("seconds", -1.0)
                val cost = if (ev.json.has("cost_usd")) ev.num("cost_usd") else null
                doneText = (if (secs >= 0) "Built in ${secs.toInt()} s" else "Built") + (cost?.let { " · $" + String.format("%.3f", it) } ?: "")
                ev.str("app")?.let { app -> R1d.main.postDelayed({ if (shell.top() === this) shell.open(Target.of("app", "id" to app)) }, 1200) }
            }
            "failed" -> {
                stage = "failed"
                error = ev.str("error") ?: "failed"
                problems = ev.json.optJSONArray("problems")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
            }
            else -> {
                stage = s ?: stage
                section = ev.str("section") ?: section
                if (ev.json.has("chars")) chars = ev.int("chars")
            }
        }
        render()
    }

    private fun start(text: String) {
        if (building) return
        brief = text.trim()
        stage = "starting"
        section = null
        chars = 0
        error = null
        problems = emptyList()
        render()
        R1d.post("/api/create", JSONObject().put("brief", brief), onError = { stage = "failed"; error = it; render() }) { buildId = it.optLong("build") }
    }

    private fun render() {
        progress.visibility = if (building) View.VISIBLE else View.GONE
        list.visibility = if (building) View.GONE else View.VISIBLE
        if (building) {
            val f = shell.focusAgent()
            worker.set(f.form, f.color, f.finish, if (stage == "done") "speaking" else "working", "build", eyes = f.eyes, wear = f.wear)
            briefText.text = "“$brief”"
            stageText.setTextColor(if (stage == "done") Theme.OK else Theme.MUTED)
            stageText.text = when (stage) {
                "done" -> doneText
                "starting" -> "Starting the agent…"
                "writing" -> "Writing ${section ?: "the spec"}…" + if (chars > 0) "\n$chars characters" else ""
                "repairing" -> "Fixing what the compiler found…"
                "installing" -> "Installing…"
                else -> stage
            }
            return
        }
        list.rebuild {
            talk("Hold the button and describe your app", Theme.accent)
            if (stage == "failed") {
                note(listOfNotNull(error).plus(problems.take(3)).joinToString("\n"), Theme.BAD, 13f)
            }
            note("Or pick one:", size = 13f)
            for (ex in EXAMPLES) item(null, ex, bold = false) { start(ex) }
        }
    }

    override fun onShow() = worker.setActive(true)
    override fun onHide() { super.onHide(); worker.setActive(false) }

    private companion object {
        val EXAMPLES = listOf(
            "Count the coffees I drink each day, with a daily limit",
            "A pushup tracker: log sets and show this week's total",
            "Track my mood from 1 to 5 each day with an optional note",
            "A tally counter I can bump up or down and reset",
        )
    }
}
