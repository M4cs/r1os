package dev.r1os.host.net

import android.os.Handler
import android.os.Looper
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.Executors

/** One event from r1d's `/ws` stream (docs/specs/r1d.md §4): the raw JSON, with typed accessors. */
class Ev(val json: JSONObject) {
    val type: String = json.optString("type")
    fun str(key: String): String? = if (json.has(key) && !json.isNull(key)) json.optString(key) else null
    fun int(key: String, def: Int = 0): Int = json.optInt(key, def)
    fun num(key: String, def: Double = 0.0): Double = json.optDouble(key, def)
    fun bool(key: String): Boolean = json.optBoolean(key)

    val gesture: String? get() = if (type == "gesture") str("kind") else null
    val isTap get() = gesture == "tap"
    val isDoubleTap get() = gesture == "double_tap"
}

/**
 * The native shell's link to r1d (ADR-0017): JSON over HTTP for actions, and `/ws` for events.
 * HTTP runs on a small pool; every callback and every event is delivered on the main thread.
 */
object R1d {
    const val HOST = "127.0.0.1"
    const val PORT = 7171
    const val BASE = "http://$HOST:$PORT"

    val main = Handler(Looper.getMainLooper())
    private val io = Executors.newFixedThreadPool(3)

    // ---- the token (ADR-0026, ADR-0036) -----------------------------------------------------------
    // r1d makes a new one each time it starts, and gives it to this app when asked (`GET /token`): it
    // checks the asking socket belongs to this app's uid. Every call carries it; a 401 means r1d
    // restarted, so it's asked for again and the call retried once.

    @Volatile private var token: String? = null
    private val tokenLock = Any()

    /** Removes the token file r1d used to write here (before ADR-0036). */
    fun init(context: android.content.Context) {
        runCatching { java.io.File(context.filesDir, "r1d-token").delete() }
    }

    /**
     * The per-boot token (asked for again when `reload`). On the main thread it never waits: it answers
     * what it has, and asks in the background.
     */
    fun token(reload: Boolean = false): String? {
        if (!reload && token != null) return token
        if (Looper.myLooper() == Looper.getMainLooper()) {
            io.execute { askToken() }
            return token
        }
        return askToken() ?: token
    }

    private fun askToken(): String? = synchronized(tokenLock) {
        runCatching {
            val c = URL("$BASE/token").openConnection() as HttpURLConnection
            c.connectTimeout = 2000
            c.readTimeout = 5000
            try {
                if (c.responseCode != 200) null
                else JSONObject(c.inputStream.bufferedReader().use { it.readText() }).optString("token").ifEmpty { null }
            } finally {
                c.disconnect()
            }
        }.getOrNull()?.also { token = it }
    }

    private fun authorize(c: HttpURLConnection, reload: Boolean = false) {
        token(reload)?.let { c.setRequestProperty("Authorization", "Bearer $it") }
    }

    // ---- HTTP -------------------------------------------------------------------------------

    fun get(path: String, onError: (String) -> Unit = {}, onResult: (JSONObject) -> Unit) =
        call("GET", path, null, onError) { onResult(it as? JSONObject ?: JSONObject()) }

    fun post(path: String, body: JSONObject = JSONObject(), onError: (String) -> Unit = {}, onResult: (JSONObject) -> Unit = {}) =
        call("POST", path, body, onError) { onResult(it as? JSONObject ?: JSONObject()) }

    fun put(path: String, body: JSONObject, onError: (String) -> Unit = {}, onResult: (JSONObject) -> Unit = {}) =
        call("PUT", path, body, onError) { onResult(it as? JSONObject ?: JSONObject()) }

    fun delete(path: String, onError: (String) -> Unit = {}, onResult: (JSONObject) -> Unit = {}) =
        call("DELETE", path, null, onError) { onResult(it as? JSONObject ?: JSONObject()) }

    fun call(method: String, path: String, body: JSONObject?, onError: (String) -> Unit, onResult: (Any?) -> Unit) {
        io.execute {
            val r = runCatching { http(method, path, body) }
            main.post { r.fold(onResult) { onError(it.message ?: "r1d isn't answering") } }
        }
    }

    /** Raw bytes (icons, images); null on any failure. */
    fun bytes(path: String, onResult: (ByteArray?) -> Unit) {
        io.execute {
            val r = runCatching {
                fun once(reload: Boolean): Pair<Int, ByteArray?> {
                    val c = URL(BASE + path).openConnection() as HttpURLConnection
                    c.connectTimeout = 3000
                    c.readTimeout = 10000
                    authorize(c, reload)
                    try {
                        val code = c.responseCode
                        return code to (if (code >= 400) null else c.inputStream.use { it.readBytes() })
                    } finally {
                        c.disconnect()
                    }
                }
                val (code, bytes) = once(false)
                if (code == 401) once(true).second else bytes
            }.getOrNull()
            main.post { onResult(r) }
        }
    }

    /** Posts raw bytes (a photo) and hands back r1d's JSON answer. */
    fun upload(path: String, bytes: ByteArray, contentType: String, onError: (String) -> Unit = {}, onResult: (JSONObject) -> Unit = {}) {
        io.execute {
            val r = runCatching {
                fun once(reload: Boolean): Pair<Int, String> {
                    val c = URL(BASE + path).openConnection() as HttpURLConnection
                    c.requestMethod = "POST"
                    c.connectTimeout = 3000
                    c.readTimeout = 30000
                    c.doOutput = true
                    c.setFixedLengthStreamingMode(bytes.size)
                    c.setRequestProperty("Content-Type", contentType)
                    authorize(c, reload)
                    try {
                        c.outputStream.use { it.write(bytes) }
                        val code = c.responseCode
                        val text = (if (code < 400) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText() }.orEmpty()
                        return code to text
                    } finally {
                        c.disconnect()
                    }
                }
                var (code, text) = once(false)
                if (code == 401) once(true).let { code = it.first; text = it.second }
                val json = runCatching { JSONObject(text) }.getOrDefault(JSONObject())
                if (code >= 400) throw IOException(json.optString("error").ifEmpty { "$path: HTTP $code" })
                json
            }
            main.post { r.fold(onResult) { onError(it.message ?: "r1d isn't answering") } }
        }
    }

    /** Headers for something that reads from r1d itself (a recording's player). */
    fun headers(): Map<String, String> = token()?.let { mapOf("Authorization" to "Bearer $it") } ?: emptyMap()

    fun enc(s: String): String = URLEncoder.encode(s, "UTF-8")

    private fun http(method: String, path: String, body: JSONObject?, reload: Boolean = false): Any? {
        val c = URL(BASE + path).openConnection() as HttpURLConnection
        try {
            c.requestMethod = method
            authorize(c, reload)
            c.connectTimeout = 3000
            c.readTimeout = 60000
            if (body != null) {
                c.doOutput = true
                c.setRequestProperty("Content-Type", "application/json")
                c.outputStream.use { it.write(body.toString().toByteArray()) }
            }
            val code = c.responseCode
            // r1d restarted with a new token: read it again, once.
            if (code == 401 && !reload) return http(method, path, body, reload = true)
            val text = (if (code < 400) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText() }.orEmpty().trim()
            val json: Any? = when {
                text.startsWith("{") -> JSONObject(text)
                text.startsWith("[") -> JSONArray(text)
                else -> null
            }
            if (code >= 400) {
                val msg = (json as? JSONObject)?.optString("error").orEmpty()
                throw IOException(msg.ifEmpty { "$path: HTTP $code" })
            }
            return json
        } finally {
            c.disconnect()
        }
    }

    // ---- events -----------------------------------------------------------------------------

    private val listeners = CopyOnWriteArraySet<(Ev) -> Unit>()
    private val connectionListeners = CopyOnWriteArraySet<(Boolean) -> Unit>()
    private var ws: WebSocketClient? = null
    private var retryMs = 500L
    private var started = false

    /** r1d's `/ws` is up. */
    var connected = false
        private set

    /** The `hello` snapshot (`GET /api/state`'s shape), updated by `motor` events. */
    var state: JSONObject? = null
        private set

    /** Subscribes to every event. Returns the unsubscribe function. */
    fun onEvent(fn: (Ev) -> Unit): () -> Unit {
        listeners.add(fn)
        return { listeners.remove(fn) }
    }

    /** Subscribes to connect/disconnect. Returns the unsubscribe function. */
    fun onConnection(fn: (Boolean) -> Unit): () -> Unit {
        connectionListeners.add(fn)
        return { connectionListeners.remove(fn) }
    }

    /** Keeps one `/ws` open, reconnecting with backoff (0.5 s, doubling to 5 s). */
    fun start() {
        if (started) return
        started = true
        connect()
    }

    /** Asks for the token first (off the main thread: r1d may have restarted with a new one). */
    private fun connect() {
        io.execute {
            val t = token(reload = true)
            main.post { open(t) }
        }
    }

    private fun open(token: String?) {
        ws = WebSocketClient(
            HOST, PORT, "/ws", token = token,
            onOpen = {
                main.post {
                    retryMs = 500
                    connected = true
                    connectionListeners.forEach { it(true) }
                }
            },
            onText = { text ->
                val msg = runCatching { JSONObject(text) }.getOrNull() ?: return@WebSocketClient
                if (msg.optString("type") == "hello") {
                    val s = msg.optJSONObject("state")
                    main.post { state = s }
                    return@WebSocketClient
                }
                val ev = Ev(msg)
                main.post {
                    if (ev.type == "motor") state?.optJSONObject("motor")?.put("deg", ev.int("deg"))
                    listeners.forEach { it(ev) }
                }
            },
            onClose = {
                main.post {
                    if (connected) {
                        connected = false
                        connectionListeners.forEach { it(false) }
                    }
                    main.postDelayed({ connect() }, retryMs)
                    retryMs = (retryMs * 2).coerceAtMost(5000)
                }
            },
        ).also { it.connect() }
    }
}

/** An agent as `/api/agents` returns it (docs/specs/agents.md §6), plus what the shell tracks. */
data class Agent(
    val id: String,
    val name: String,
    val form: String,
    val color: String,
    val finish: String,
    val role: String,
    val brain: String,
    var state: String,
    var chat: String?,
    var unread: Int,
    /** Its last tool while working (from `chat` tool events). */
    var tool: String? = null,
    /** All of it as r1d sent it (personality, custom…): what a PUT sends back, changed. */
    val raw: JSONObject = JSONObject(),
    /** The agent whose message it's answering now (ADR-0012), not the user. */
    var by: String? = null,
    /** The newest message from another agent you haven't seen: `{from, kind, text}`. */
    var mail: JSONObject? = null,
    /** Its eyes, and what it wears (ADR-0043). */
    val eyes: String = "classic",
    val wear: List<String> = emptyList(),
    /** The linked computer it works on (its name), if not the R1 (docs/specs/companion.md §7.2), and whether it's there. */
    val computer: String? = null,
    val computerHere: Boolean = true,
) {
    companion object {
        fun from(j: JSONObject) = Agent(
            id = j.optString("id"),
            name = j.optString("name", "Pip"),
            form = j.optString("form", "orb"),
            color = j.optString("color", "orange"),
            finish = j.optString("finish", "soft"),
            role = j.optString("role"),
            brain = j.optString("brain", "claude"),
            state = j.optString("state", "idle"),
            chat = j.optString("chat").ifEmpty { null },
            unread = j.optInt("unread"),
            raw = j,
            by = j.optString("by").ifEmpty { null },
            mail = j.optJSONObject("mail"),
            eyes = j.optString("eyes", "classic").ifEmpty { "classic" },
            wear = j.optJSONArray("wear")?.let { w -> (0 until w.length()).map { w.optString(it) } }.orEmpty(),
            computer = j.optJSONObject("computer")?.let { it.optString("name").ifEmpty { "a computer" } },
            computerHere = j.optJSONObject("computer")?.optBoolean("connected", true) ?: true,
        )

        val DEFAULT = Agent("", "Pip", "orb", "orange", "soft", "", "claude", "idle", null, 0)
    }
}

/**
 * `agent`'s current conversation: the chat you had with it most lately, however long ago (ADR-0057, T-105). Talking
 * carries it on; a new chat starts only when you start one (New chat in its space, or the companion). Null when it has
 * none yet (= a new chat).
 */
fun currentChat(agent: String, onResult: (String?) -> Unit) {
    R1d.get("/api/chats?agent=${R1d.enc(agent)}", onError = { onResult(null) }) { r ->
        onResult(r.optJSONArray("chats")?.optJSONObject(0)?.optString("id")?.ifEmpty { null })
    }
}
