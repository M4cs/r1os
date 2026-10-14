package dev.r1os.host.shell

import android.view.Gravity
import android.widget.FrameLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import org.json.JSONArray
import org.json.JSONObject

/**
 * Settings › Connections (R8, ADR-0052): the apps your agents reach on your accounts. Claude agents have the connectors
 * you added on claude.ai (Gmail, Google Calendar, Drive…), Codex agents the apps on your ChatGPT account. r1os keeps no
 * passwords or tokens: they stay with claude.ai and ChatGPT. Each is on for every agent unless you turn it off for one;
 * reading runs, and anything that acts asks on the R1 (allow, always allow, don't allow).
 */
class ConnectionsScreen(shell: Shell) : ListPage(shell, "Connections") {
    private var state: JSONObject? = null
    /** The agents' skills (T-086): `GET /api/skills`. */
    private var skills: JSONObject? = null
    private var looking = false

    init {
        list.gap(6f)
        render()
    }

    private fun load() {
        R1d.get("/api/connections") { state = it; render() }
        R1d.get("/api/skills") { skills = it; render() }
    }

    /** Looks again (the CLI's list: a few seconds, no model call). */
    private fun check() {
        looking = true
        render()
        R1d.post("/api/connections/check", JSONObject(), onError = { looking = false; render() }) { state = it; looking = false; render() }
    }

    override fun onShow() {
        load()
        check()
    }

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "connections_changed" && !looking) load()
    }

    private fun render() {
        val keep = list.focus
        list.rebuild {
            note("Your agents use the apps you connect to your accounts. r1os keeps no passwords or tokens: claude.ai and ChatGPT do.", size = 14f)
            section("claude agents · claude.ai")
            val servers = state?.optJSONArray("servers").objects()
            if (servers.isEmpty()) note(if (looking || state == null) "looking…" else "none yet")
            for (s in servers) {
                val ok = s.optString("status") == "connected"
                item("plugs-connected", s.optString("label"), if (ok) accent else null, sub = connectionWords(s), trailing = if (s.optString("status") == "needs-auth") "sign in" else null) {
                    shell.push(ConnectionPage(shell, s.optString("name")))
                }
            }
            item("plus", "add gmail, calendar and more", sub = "on claude.ai, from your phone") {
                shell.push(LinkQrScreen(shell, "Add a connection", state?.optString("add_url").orEmpty(), "Scan with your phone and add one on claude.ai. Your Claude agents have it from their next chat."))
            }
            section("codex agents · chatgpt")
            item("plus", "chatgpt apps", sub = "the apps on your ChatGPT account, as they are") {
                shell.push(LinkQrScreen(shell, "ChatGPT apps", state?.optString("chatgpt_url").orEmpty(), "Scan with your phone and connect apps on chatgpt.com. Your Codex agents use them as you set them up there."))
            }
            if (looking && servers.isNotEmpty()) note("looking again…", size = 13f)
            // Skills (T-086): instructions (and sometimes scripts) for a kind of task, for every agent on the R1.
            section("skills")
            val all = skills?.optJSONArray("skills").objects()
            if (all.isEmpty()) note(if (skills == null) "looking…" else "none yet")
            for (k in all) {
                val on = k.optBoolean("on")
                item("puzzle-piece", k.optString("name"), if (on) accent else null, sub = if (k.optString("from") == "claude.ai") "from claude.ai" else "added on the r1", trailing = if (on) "on" else "off") {
                    shell.push(SkillPage(shell, k.optString("name")))
                }
            }
            note("To add one, ask an agent (“add the pdf skill from anthropics/skills”), or paste its link in the companion.", size = 13f)
        }
        if (keep >= 0 && list.size > 0) list.setFocus(keep.coerceAtMost(list.size - 1), animate = false)
    }
}

/** "connected · reads 4, asks for 4", "needs signing in", "couldn't connect". */
private fun connectionWords(s: JSONObject): String {
    val reads = s.optJSONArray("reads")?.length() ?: 0
    val acts = s.optJSONArray("acts")?.length() ?: 0
    return when (s.optString("status")) {
        "connected" -> if (reads + acts == 0) "connected" else "connected · reads $reads, asks for $acts"
        "needs-auth" -> "needs signing in"
        "failed" -> "couldn't connect"
        else -> "connecting"
    }
}

private fun JSONArray?.objects(): List<JSONObject> = (0 until (this?.length() ?: 0)).mapNotNull { this?.optJSONObject(it) }
private fun JSONArray?.strings(): List<String> = (0 until (this?.length() ?: 0)).mapNotNull { this?.optString(it) }

/** One connection: signing in, what it does, which agents have it, and what they may do without asking. */
class ConnectionPage(shell: Shell, private val server: String) : ListPage(shell, server.removePrefix("claude.ai ")) {
    private var state: JSONObject? = null
    private var signing = false
    private var said: String? = null

    init {
        list.gap(6f)
        load()
    }

    private fun load() = R1d.get("/api/connections") { state = it; render() }

    override fun onShow() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "connections_changed") load()
    }

    private fun signIn() {
        signing = true
        render()
        R1d.post("/api/connections/login", JSONObject().put("server", server), onError = { signing = false; said = it; render() }) { r ->
            signing = false
            render()
            shell.push(LinkQrScreen(shell, "Sign in", r.optString("url"), "Scan with your phone and sign in on claude.ai. Your agents have it from their next chat."))
        }
    }

    private fun render() {
        val keep = list.focus
        val st = state ?: return
        val s = st.optJSONArray("servers").objects().firstOrNull { it.optString("name") == server }
        val slug = s?.optString("slug").orEmpty()
        list.rebuild {
            if (s == null) {
                note("It isn't there any more: removed on claude.ai.")
                return@rebuild
            }
            said?.let { note(it, Theme.BAD) }
            val status = s.optString("status")
            // One of yours, added from the computer with a key: nothing to sign in to.
            val yours = s.optString("source") != "claudeai"
            item(if (status == "connected") "check-circle" else "warning-circle", connectionWords(s), if (status == "connected") accent else null,
                sub = if (yours) "yours, added with a key from your computer" else if (signing) "getting the link…" else if (status == "connected") "sign in again" else "sign in on claude.ai") {
                if (!signing && !yours) signIn()
            }
            val reads = s.optJSONArray("reads").strings()
            val acts = s.optJSONArray("acts").strings()
            if (reads.isEmpty() && acts.isEmpty()) {
                note("What it can do shows once an agent's chat has started with it.", size = 14f)
            } else {
                section("what it does")
                if (reads.isNotEmpty()) note("reads, without asking: ${reads.joinToString(", ") { it.replace('_', ' ') }}", size = 14f)
                if (acts.isNotEmpty()) note("asks you first: ${acts.joinToString(", ") { it.replace('_', ' ') }}", size = 14f)
            }
            val agents = st.optJSONArray("agents").objects().filter { it.optString("brain") == "claude" }
            if (agents.isNotEmpty()) section("claude agents")
            for (a in agents) {
                val on = a.optJSONArray("off").strings().none { it == server }
                val ag = shell.agent(a.optString("id"))
                item("user-circle", a.optString("name"), if (on) ag?.let { Theme.colorOf(it.color) } ?: accent else null, trailing = if (on) "on" else "off") {
                    R1d.post("/api/connections/agent", JSONObject().put("agent", a.optString("id")).put("server", server).put("on", !on), onError = { said = it; render() }) { state = it; render() }
                }
            }
            // "Always allow" answers on the approval card, for this connection's tools: a click asks again.
            val always = agents.flatMap { a -> a.optJSONArray("always").strings().filter { it.startsWith("mcp__${slug}__") }.map { a to it } }
            if (always.isNotEmpty()) section("always allowed")
            for ((a, tool) in always) {
                val short = tool.removePrefix("mcp__${slug}__").replace('_', ' ')
                item("seal-check", "${a.optString("name")}: $short", sub = "click to ask again") {
                    R1d.post("/api/connections/always", JSONObject().put("agent", a.optString("id")).put("tool", tool).put("allow", false), onError = { said = it; render() }) { state = it; render() }
                }
            }
        }
        if (keep >= 0 && list.size > 0) list.setFocus(keep.coerceAtMost(list.size - 1), animate = false)
    }
}

/**
 * One skill (T-086): what it's for, on or off for every agent and for each, and removing one added on the R1 (click
 * twice). The claude.ai account's stay: they can only be turned off here.
 */
class SkillPage(shell: Shell, private val name: String) : ListPage(shell, name) {
    private var state: JSONObject? = null
    private var said: String? = null
    private val removing = Confirm { render() }

    init {
        list.gap(6f)
        load()
    }

    private fun load() = R1d.get("/api/skills") { state = it; render() }

    override fun onShow() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "connections_changed") load()
    }

    private fun set(agent: String?, on: Boolean) {
        val body = JSONObject().put("skill", name).put("on", on)
        if (agent != null) body.put("agent", agent)
        R1d.post("/api/skills/on", body, onError = { said = it; render() }) { state = it; render() }
    }

    private fun render() {
        val keep = list.focus
        val st = state ?: return
        val k = st.optJSONArray("skills").objects().firstOrNull { it.optString("name") == name }
        list.rebuild {
            if (k == null) {
                note("It isn't there any more.")
                return@rebuild
            }
            said?.let { note(it, Theme.BAD) }
            k.optString("description").takeIf { it.isNotBlank() }?.let { note(it, Theme.FG, 15f) }
            val claudeOnly = k.optString("from") == "claude.ai"
            note(if (claudeOnly) "from your claude.ai account · for claude agents" else "added on the r1 · for claude and codex agents", size = 13f)
            val on = k.optBoolean("on")
            item(if (on) "check-circle" else "circle", if (on) "on for your agents" else "off for every agent", if (on) accent else null, trailing = if (on) "on" else "off") {
                set(null, !on)
            }
            if (on) {
                val agents = st.optJSONArray("agents").objects().filter { !claudeOnly || it.optString("brain") == "claude" }
                if (agents.isNotEmpty()) section("agents")
                for (a in agents) {
                    val mine = a.optJSONArray("off").strings().none { it == name }
                    val ag = shell.agent(a.optString("id"))
                    item("user-circle", a.optString("name"), if (mine) ag?.let { Theme.colorOf(it.color) } ?: accent else null, trailing = if (mine) "on" else "off") {
                        set(a.optString("id"), !mine)
                    }
                }
            }
            if (!claudeOnly) {
                section("")
                val armed = removing.armed == "remove"
                item("trash", if (armed) "click again to remove" else "remove", if (armed) Theme.BAD else null) {
                    removing.tap("remove") {
                        R1d.delete("/api/skills/${R1d.enc(name)}", onError = { said = it; render() }) { shell.flash("removed"); shell.back() }
                    }
                }
            }
        }
        if (keep >= 0 && list.size > 0) list.setFocus(keep.coerceAtMost(list.size - 1), animate = false)
    }
}

/** A link to open on your phone, as a QR code: adding or signing in to a connection on claude.ai or chatgpt.com. */
class LinkQrScreen(shell: Shell, title: String, private val url: String, private val words: String) : ListPage(shell, title) {
    init {
        list.rebuild {
            place(FrameLayout(context).apply {
                addView(QrView(context, url), FrameLayout.LayoutParams(context.dpi(200f), context.dpi(200f), Gravity.CENTER))
            })
            note(words, Theme.FG, 16f, center = true, weight = 600)
            note(url.substringAfter("://").substringBefore("/"), size = 13f, center = true)
        }
    }
}
