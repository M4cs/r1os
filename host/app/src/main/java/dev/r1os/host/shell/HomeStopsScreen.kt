package dev.r1os.host.shell

import dev.r1os.host.AndroidApps
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import org.json.JSONArray
import org.json.JSONObject

/**
 * Settings › Home (T-109): what's on home's wheel, in order, arranged with the wheel and the button. Your agents, then
 * what you pinned (ADR-0044): a click lifts one, the wheel moves it among its kind, a click keeps it there and a double
 * click puts it back. Then everything that can be pinned (your apps, r1os's tools, Android apps), each on or off home.
 * On home itself the same is a long press (edit mode, the launcher), which the wheel and the button can't do.
 */
class HomeStopsScreen(shell: Shell) : ListPage(shell, "home") {
    private var pins: List<String>? = null
    private var apps: List<JSONObject> = emptyList()
    private val android = AndroidApps.list(context).filter { !it.system }
    /** A stop being moved: its kind (`agents` or `pins`), the order so far, and the one lifted. */
    private var moving: Moving? = null
    /** The agents' new order, until r1d's `agents_changed` brings it (no flick back to the old one meanwhile). */
    private var agentOrder: List<String>? = null
    private var msg: String? = null

    private class Moving(val kind: String, val order: MutableList<String>, val id: String)

    init {
        fetch("/api/home", ::render) { pins = strings(it, "pins") }
        fetch("/api/apps", ::render) { r -> apps = r.optJSONArray("apps")?.let { a -> (0 until a.length()).mapNotNull { a.optJSONObject(it) } }.orEmpty() }
        render()
    }

    override fun onAgents() {
        agentOrder = null
        render()
    }

    private fun render(): Unit = list.rebuild {
        note("Home's wheel, in order: your agents, then what you pinned. Click one to move it with the wheel.", size = 13f)
        val m = moving
        val agents = if (m?.kind == "agents") m.order else agentOrder ?: shell.agents.map { it.id }
        section("agents")
        for (id in agents) {
            val a = shell.agent(id) ?: continue
            val lifted = m?.id == id
            item("smiley", if (lifted) "‹ ${a.name} ›" else a.name, Theme.colorOf(a.color), sub = if (lifted) "turn to move · click to keep" else null) {
                lift("agents", agents, id)
            }.tag = "agent:$id"
        }
        val ps = if (m?.kind == "pins") m.order else pins.orEmpty()
        if (ps.isNotEmpty()) section("pinned after them")
        for (p in ps) {
            val s = stop(p) ?: continue
            val lifted = m?.id == p
            item(s.icon, if (lifted) "‹ ${s.name} ›" else s.name, if (lifted) accent else null, image = s.image,
                sub = if (lifted) "turn to move · click to keep" else s.kind) { lift("pins", ps, p) }.tag = "pin:$p"
        }
        val on = pins ?: return@rebuild
        section("your apps")
        for (a in apps) pinRow(a.optString("id"), a.optString("name"), a.optString("icon", "app-window"), null, on)
        section("tools")
        for ((id, name, icon) in HomeScreen.TOOLS) pinRow(HomeScreen.TOOL + id, name, icon, null, on)
        if (android.isNotEmpty()) section("android apps")
        for (a in android) pinRow("android:${a.pkg}", a.name, null, a.icon(context), on)
        msg?.let { note(it) }
    }

    private fun FocusList.pinRow(p: String, name: String, icon: String?, image: android.graphics.drawable.Drawable?, on: List<String>) {
        val pinned = p in on
        item(icon, name, if (pinned) accent else null, image = image, trailing = if (pinned) "on home" else "off") { toggle(p) }.tag = "on:$p"
    }

    private class Stop(val name: String, val icon: String?, val image: android.graphics.drawable.Drawable?, val kind: String)

    /** A pin's name and icon: one of your apps, a tool (`tool:`), or an Android app (`android:`). */
    private fun stop(p: String): Stop? = when {
        p.startsWith(HomeScreen.TOOL) -> HomeScreen.TOOLS.firstOrNull { it.first == p.removePrefix(HomeScreen.TOOL) }?.let { Stop(it.second, it.third, null, "a tool") }
        p.startsWith("android:") -> android.firstOrNull { it.pkg == p.removePrefix("android:") }?.let { Stop(it.name, null, it.icon(context), "an android app") }
        else -> apps.firstOrNull { it.optString("id") == p }?.let { Stop(it.optString("name"), it.optString("icon", "app-window"), null, "an app") }
    }

    private fun lift(kind: String, order: List<String>, id: String) {
        msg = null
        moving = Moving(kind, order.toMutableList(), id)
        Sounds.play(Sounds.S.TICK)
        render()
    }

    /** While one is lifted: the wheel moves it, a click keeps it, a double click puts it back. */
    override fun extra(ev: Ev): Boolean {
        val m = moving ?: return false
        when {
            ev.type == "wheel" -> {
                val i = m.order.indexOf(m.id)
                val j = i + if (ev.int("delta") > 0) 1 else -1
                if (j in m.order.indices) {
                    m.order[i] = m.order[j].also { m.order[j] = m.order[i] }
                    Sounds.play(Sounds.S.TICK)
                    render()
                }
            }
            ev.isTap -> keep(m)
            ev.isDoubleTap -> {
                moving = null
                Sounds.play(Sounds.S.BACK)
                render()
            }
            else -> return false
        }
        return true
    }

    private fun keep(m: Moving) {
        moving = null
        val fail = { e: String -> msg = e; render() }
        if (m.kind == "agents") {
            agentOrder = m.order
            R1d.put("/api/agents/order", JSONObject().put("ids", JSONArray(m.order)), onError = { agentOrder = null; fail(it) })
        } else {
            pins = m.order
            R1d.put("/api/home", JSONObject().put("pins", JSONArray(m.order)), onError = fail) { pins = strings(it, "pins"); render() }
        }
        Sounds.play(Sounds.S.POP)
        render()
    }

    /** On home or off it: on goes last (12 at most, ADR-0044). */
    private fun toggle(p: String) {
        val now = pins ?: return
        val next = if (p in now) now - p else now + p
        R1d.put("/api/home", JSONObject().put("pins", JSONArray(next)), onError = { msg = it; render() }) { r ->
            msg = null
            pins = strings(r, "pins")
            render()
        }
    }

    private fun strings(r: JSONObject, key: String): List<String> = r.optJSONArray(key)?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
}
