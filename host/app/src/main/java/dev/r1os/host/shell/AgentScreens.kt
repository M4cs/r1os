package dev.r1os.host.shell

import android.graphics.Color
import android.text.TextUtils
import android.view.Gravity
import android.widget.LinearLayout
import dev.r1os.host.net.Agent
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.net.currentChat
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject

// An agent's space and what hangs off it (docs/specs/experience.md §2.7, ADR-0011; ported from
// shell/src/space.tsx): its chats, memory, look and brain, managing its chats, and making a new agent.

private val BRAIN = mapOf("claude" to "Claude", "codex" to "Codex")

private val LIVE = mapOf(
    "waiting" to "Waiting for room to run…", "thinking" to "Thinking…", "speaking" to "Talking…",
    "working" to "Working on it…", "error" to "Something went wrong",
)

private fun chatsOf(r: JSONObject): List<JSONObject> {
    val a = r.optJSONArray("chats") ?: JSONArray()
    return (0 until a.length()).mapNotNull { a.optJSONObject(it) }
}

/** The chats an agent's space lists before Older chats. */
private const val SHOWN = 8

/**
 * Everything that belongs to one agent (double click on home, or touch it):
 *
 *   New chat · its chats, newest first (the newest 8, then Older chats) · Messages (its threads with other agents, ADR-0012) · Memory ·
 *   Look and personality · Talks with Claude/Codex · Manage chats · Move earlier/later · Remove
 *
 * Hold talks into the chat under the focus (New chat: a new one), else its current chat.
 */
class SpaceScreen(shell: Shell, private val agentKey: String) : ListPage(shell, shell.agent(agentKey)?.name ?: "", Theme.colorOf(shell.agent(agentKey)?.color)) {
    override val agentId get() = agentKey
    private var chats: List<JSONObject>? = null
    /** Its threads with other agents (ADR-0012). */
    private var threads: List<JSONObject> = emptyList()
    private var memories: Int? = null
    private var acc: JSONObject? = null
    private var msg: String? = null
    private val confirm: Confirm = Confirm { render() }
    private val agent: Agent get() = shell.agent(agentKey) ?: Agent.DEFAULT

    init {
        loadChats()
        fetch("/api/memories?agent=${R1d.enc(agentKey)}", ::render) { r ->
            val ms = r.optJSONArray("memories") ?: JSONArray()
            memories = (0 until ms.length()).count { ms.optJSONObject(it)?.optBoolean("shared") == false }
        }
        fetch("/api/accounts", ::render) { acc = it.optJSONObject("accounts") }
        render()
    }

    private fun loadChats() {
        fetch("/api/chats?agent=${R1d.enc(agentKey)}", ::render, onError = { if (chats == null) { chats = emptyList(); soon(::render) } }) { chats = chatsOf(it) }
        fetch("/api/chats?agent=${R1d.enc(agentKey)}&kind=thread", ::render) { threads = chatsOf(it) }
    }

    override fun onAgents() {
        if (shell.agent(agentKey) == null) return // removed: the remove itself goes home
        header.setTitle(agent.name)
        render()
    }

    override fun onAnyEvent(ev: Ev) {
        val mine = ev.str("agent") == agentKey
        when {
            ev.type == "chat" && mine && ev.str("kind") in setOf("done", "error", "note", "mail") -> loadChats()
            ev.type == "unread" && mine -> loadChats()
            // Start the agent while the user talks: in the chat under the focus.
            ev.type == "voice" && ev.str("state") == "listening" && shell.top() === this -> target { c ->
                R1d.post("/api/chats/warm", JSONObject().put("agent", agentKey).put("chat", c ?: JSONObject.NULL))
            }
        }
    }

    /** The chat under the focus: its id; null for New chat (a new one) or another row (the current one). */
    private fun target(onResult: (String?) -> Unit) {
        when (val tag = list.focused()?.tag as? String) {
            "new" -> onResult(null)
            null -> currentChat(agentKey, onResult)
            else -> onResult(tag)
        }
    }

    override fun extra(ev: Ev): Boolean {
        if (ev.type == "voice" && ev.str("state") == "final") {
            val say = ev.str("text")?.trim().orEmpty()
            if (say.isEmpty()) return false
            target { c -> shell.open(Target.of("chat", "agent" to agentKey, "id" to c, "say" to say)) }
            return true
        }
        return false
    }

    private fun render() {
        val a = agent
        val other = if (a.brain == "claude") "codex" else "claude"
        val at = shell.agents.indexOfFirst { it.id == agentKey }
        list.rebuild {
            // The head: the agent, what it's for, what it's doing.
            val head = LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                clipChildren = false
                setPadding(context.dpi(4f), 0, 0, context.dpi(6f))
                addView(CharacterView(context, context.dpi(56f)).apply { set(a.form, a.color, a.finish, a.state, CharacterView.activityOf(a.tool)) },
                    LinearLayout.LayoutParams(context.dpi(56f), context.dpi(56f)))
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.VERTICAL
                    setPadding(context.dpi(12f), 0, 0, 0)
                    addView(context.text(16f, 650).apply { text = if (a.role.isNotEmpty()) "For ${a.role}" else "A bit of everything"; maxLines = 1; ellipsize = TextUtils.TruncateAt.END })
                    addView(context.text(13f, 500, Theme.MUTED).apply { text = LIVE[a.state].orEmpty(); visibility = if (LIVE[a.state] == null) android.view.View.GONE else android.view.View.VISIBLE })
                }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            }
            place(head)
            item("plus", "New chat", Theme.colorOf(a.color), bold = true) { shell.open(Target.of("chat", "agent" to agentKey)) }.tag = "new"
            val cs = chats
            if (!cs.isNullOrEmpty()) section("Chats")
            // The newest few: the rest are a click away, and every row is ~5 ms of the R1's main thread (T-106).
            cs?.take(SHOWN)?.forEach { c ->
                val cid = c.optString("id")
                val unread = c.optInt("unread")
                val badge = if (unread > 0) context.text(12f, 800, Color.parseColor("#111111")).apply {
                    text = unread.toString()
                    gravity = Gravity.CENTER
                    minWidth = context.dpi(22f)
                    setPadding(context.dpi(6f), context.dpi(3f), context.dpi(6f), context.dpi(3f))
                    background = roundRect(Theme.colorOf(a.color), context.dpi(11f).toFloat())
                } else null
                val busy = a.chat == cid && a.state != "idle"
                item(
                    null, c.optString("title"), bold = unread > 0, sub = c.optString("last").ifEmpty { null },
                    trailing = if (busy) "…" else ago(c.optLong("updated")), trailingView = badge,
                ) { shell.open(Target.of("chat", "agent" to agentKey, "id" to cid)) }.tag = cid
            }
            if (cs != null && cs.size > SHOWN) item("clock-counter-clockwise", "Older chats", trailing = "${cs.size - SHOWN}") {
                shell.push(OlderChatsScreen(shell, agentKey, SHOWN))
            }.tag = "older"
            if (cs != null && cs.isEmpty()) note("No chats yet.")

            // What it and the other agents said to each other: read here, it isn't a chat with you.
            if (threads.isNotEmpty()) section("Messages")
            threads.take(10).forEach { t ->
                val tid = t.optString("id")
                val peer = shell.agent(t.optString("peer"))
                val unread = t.optInt("unread")
                val badge = if (unread > 0) context.text(12f, 800, Color.parseColor("#111111")).apply {
                    text = unread.toString()
                    gravity = Gravity.CENTER
                    minWidth = context.dpi(22f)
                    setPadding(context.dpi(6f), context.dpi(3f), context.dpi(6f), context.dpi(3f))
                    background = roundRect(Theme.colorOf(peer?.color), context.dpi(11f).toFloat())
                } else null
                val busy = a.chat == tid && a.state != "idle"
                item(
                    "chats-circle", "with ${peer?.name ?: t.optString("title")}", Theme.colorOf(peer?.color), bold = unread > 0,
                    sub = t.optString("last").ifEmpty { null }, trailing = if (busy) "…" else ago(t.optLong("updated")), trailingView = badge,
                ) { shell.open(Target.of("chat", "agent" to agentKey, "id" to tid)) }.tag = tid
            }

            section(a.name)
            item("sparkle", "Memory", trailing = memories?.toString() ?: "") { shell.open(Target.of("memory", "agent" to agentKey)) }
            item("palette", "Look and personality") { shell.open(Target.of("agent-look", "id" to agentKey)) }
            // What it may change on the R1, and whether it asks first (ADR-0045).
            val can = a.raw.optJSONObject("can")
            val allowed = listOf("look", "device", "apps").count { can?.optString(it) == "allow" }
            item("shield-check", "What it can change", trailing = if (allowed > 0) "$allowed allowed" else "asks you") {
                shell.open(Target.of("agent-can", "id" to agentKey))
            }
            item("lightning", "talks with ${BRAIN[a.brain]}", trailing = if (acc?.optBoolean(other) == true) "switch to ${BRAIN[other]}" else "") { switchBrain(other) }
            // Where it works (docs/specs/companion.md §7.2): the R1, or a linked computer.
            item("desktop", if (a.computer != null) "works on ${a.computer}" else "works on the R1", sub = if (a.computer != null && !a.computerHere) "not linked now" else null) {
                shell.open(Target.of("agent-where", "id" to agentKey))
            }
            if (!cs.isNullOrEmpty()) item("list", "Manage chats") { shell.open(Target.of("chats", "agent" to agentKey)) }
            if (at > 0) item("arrow-up", "Move earlier on the wheel") { move(at, -1) }
            if (at >= 0 && at < shell.agents.size - 1) item("arrow-down", "Move later on the wheel") { move(at, 1) }
            if (shell.agents.size > 1) {
                val armed = confirm.armed == "remove"
                item("trash", if (armed) "click again: remove ${a.name}, its chats and memory" else "remove ${a.name}", danger = armed) { remove() }
            }
            msg?.let { note(it) }
        }
    }

    private fun switchBrain(other: String) {
        val a = agent
        if (acc?.optBoolean(other) != true) {
            msg = "${BRAIN[other]} isn't connected (Settings › Accounts)."
            render()
            return
        }
        R1d.put("/api/agents/${R1d.enc(agentKey)}", JSONObject(a.raw.toString()).put("brain", other), onError = { msg = it; render() }) {
            msg = "New chats with ${a.name} use ${BRAIN[other]}. Older chats keep theirs."
            render()
        }
    }

    private fun move(at: Int, d: Int) {
        val ids = shell.agents.map { it.id }.toMutableList()
        val j = at + d
        if (j !in ids.indices) return
        ids[at] = ids[j].also { ids[j] = ids[at] }
        R1d.put("/api/agents/order", JSONObject().put("ids", JSONArray(ids)))
    }

    private fun remove() = confirm.tap("remove") {
        R1d.delete("/api/agents/${R1d.enc(agentKey)}", onError = { msg = it; render() }) { shell.toHome() }
    }

    override fun onDestroy() = confirm.cancel()
}

/** An agent's chats after the newest [skip] its space shows, oldest last: a click opens one (T-106). */
class OlderChatsScreen(shell: Shell, private val agentKey: String, private val skip: Int) :
    ListPage(shell, "older chats", Theme.colorOf(shell.agent(agentKey)?.color)) {
    override val agentId get() = agentKey
    private var chats: List<JSONObject>? = null

    init {
        fetch("/api/chats?agent=${R1d.enc(agentKey)}", ::render) { chats = chatsOf(it) }
        render()
    }

    private fun render(): Unit = list.rebuild {
        val cs = chats?.drop(skip)
        if (cs == null) { note("…"); return@rebuild }
        if (cs.isEmpty()) note("No older chats.")
        for (c in cs) {
            val cid = c.optString("id")
            item(null, c.optString("title"), bold = c.optInt("unread") > 0, sub = c.optString("last").ifEmpty { null }, trailing = ago(c.optLong("updated"))) {
                shell.open(Target.of("chat", "agent" to agentKey, "id" to cid))
            }.tag = cid
        }
    }
}

/** An agent's chats, to delete old ones (click twice). Opening and resuming is in its space. */
class ManageChatsScreen(shell: Shell, private val agentKey: String) : ListPage(shell, "chats with ${shell.agent(agentKey)?.name ?: ""}", Theme.colorOf(shell.agent(agentKey)?.color)) {
    override val agentId get() = agentKey
    private var chats: List<JSONObject>? = null
    private var msg: String? = null
    private val confirm: Confirm = Confirm { render() }

    init {
        load()
        render()
    }

    private fun load(): Unit = R1d.get("/api/chats?agent=${R1d.enc(agentKey)}", onError = { chats = chats ?: emptyList(); render() }) { chats = chatsOf(it); render() }

    private fun render(): Unit = list.rebuild {
        note("Click a chat twice to delete it.", size = 13f)
        val cs = chats
        if (cs != null && cs.isEmpty()) note("No chats with ${shell.agent(agentKey)?.name ?: "this agent"}.")
        cs?.forEach { c ->
            val cid = c.optString("id")
            val armed = confirm.armed == cid
            item(null, if (armed) "click again to delete" else c.optString("title"), trailing = ago(c.optLong("updated")), danger = armed) {
                confirm.tap(cid) {
                    R1d.delete("/api/chats/${R1d.enc(cid)}", onError = { msg = it; render() }) {
                        msg = "Deleted “${c.optString("title")}”."
                        load()
                    }
                }
            }
        }
        msg?.let { note(it) }
    }

    override fun onDestroy() = confirm.cancel()
}

/**
 * What's remembered. With an agent: its own facts, then the facts every agent shares. Without: the
 * shared facts alone (Settings › About you). Click a fact twice to forget it.
 */
class MemoryScreen(shell: Shell, private val agentKey: String?) :
    ListPage(shell, if (agentKey == null) "about you" else "${shell.agent(agentKey)?.name ?: ""}'s memory", Theme.colorOf(shell.agent(agentKey)?.color)) {
    override val agentId get() = agentKey
    private var ms: List<JSONObject>? = null
    private val confirm: Confirm = Confirm { render() }

    init {
        load()
        render()
    }

    private fun load(): Unit = R1d.get("/api/memories" + (agentKey?.let { "?agent=${R1d.enc(it)}" } ?: ""), onError = { ms = emptyList(); render() }) { r ->
        val a = r.optJSONArray("memories") ?: JSONArray()
        ms = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
        render()
    }

    private fun render(): Unit = list.rebuild {
        val all = ms ?: emptyList()
        val own = all.filter { !it.optBoolean("shared") }
        val shared = all.filter { it.optBoolean("shared") }
        if (all.isNotEmpty()) note("Click a fact twice to forget it.", size = 13f)
        val row = { m: JSONObject ->
            val key = m.optLong("id").toString()
            val armed = confirm.armed == key
            val by = shell.agent(m.optString("by"))?.name
            item(null, if (armed) "click again to forget" else m.optString("text"), sub = if (!armed && m.optBoolean("shared") && by != null) "from $by" else null, danger = armed, bold = false) {
                confirm.tap(key) { R1d.delete("/api/memories/$key") { load() } }
            }
            Unit
        }
        if (agentKey != null) {
            section("Only ${shell.agent(agentKey)?.name ?: ""}")
            if (own.isEmpty()) note("Nothing of its own yet.") else own.forEach(row)
        }
        section("Every agent knows")
        if (shared.isEmpty()) note("Nothing yet. Tell any agent about yourself.") else shared.forEach(row)
    }

    override fun onDestroy() = confirm.cancel()
}

/** The end of the home wheel: design a new agent. It lands on the wheel just before New agent. */
fun newAgentPage(shell: Shell): Screen {
    val initial = DesignerPanel.freshAgent(shell.agents)
    val panel = DesignerPanel(shell, initial, "Make it",
        onSave = { a, ok, fail ->
            // It talks with whichever account is connected (Claude if both, or neither).
            R1d.get("/api/accounts", onError = { fail(it) }) { r ->
                val acc = r.optJSONObject("accounts")
                val brain = if (acc?.optBoolean("claude") == true || acc?.optBoolean("codex") != true) "claude" else "codex"
                val body = JSONObject(a.toString()).put("brain", brain)
                body.remove("id")
                R1d.post("/api/agents", body, onError = fail) { ok(it.optJSONObject("agent") ?: JSONObject()) }
            }
        },
        onDone = { made -> shell.open(Target.of("home", "at" to made.optString("id"))) },
    )
    return PanelPage(shell, "New agent", Theme.colorOf(initial.optString("color")), panel)
}

/** Redesign an agent: the same steps as making one. */
fun agentLookPage(shell: Shell, id: String): Screen? {
    val a = shell.agent(id) ?: return null
    val panel = DesignerPanel(shell, a.raw, "Save",
        onSave = { x, ok, fail -> R1d.put("/api/agents/${R1d.enc(id)}", x, onError = fail) { ok(it.optJSONObject("agent") ?: x) } },
        onDone = { shell.back() },
    )
    return PanelPage(shell, "Look and personality", Theme.colorOf(a.color), panel, agentId = id)
}
