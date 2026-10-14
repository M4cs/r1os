package dev.r1os.host.shell

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.camera.Motor
import dev.r1os.host.camera.Viewfinder
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream

/**
 * Settings › Computers (ADR-0048, docs/specs/companion.md §3): the computers this R1 is linked to, whether each is
 * connected now (and how: direct or through a relay), and **link a computer**. A click on one tries it (round trips);
 * **unlink** under it, clicked twice, unlinks it (T-006: it was a long press, which the wheel and button can't do).
 */
class ComputersScreen(shell: Shell) : ListPage(shell, "Computers") {
    private var computers: List<JSONObject> = emptyList()
    private var loaded = false
    private var msg: String? = null
    private val confirm = Confirm { render() }

    init {
        render()
    }

    override fun onShow() = load()

    private fun load() = R1d.get("/api/link") { r ->
        val a = r.optJSONArray("computers") ?: JSONArray()
        computers = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
        loaded = true
        render()
    }

    private fun render(): Unit = list.rebuild {
        item("plus", "link a computer", accent, sub = "point the camera at its QR") { shell.push(ScanScreen(shell)) }
        if (computers.isNotEmpty()) section("linked")
        for (c in computers) {
            val id = c.optString("id")
            val path = c.optString("path").takeIf { c.optBoolean("connected") && it.isNotEmpty() && it != "null" }
            item("desktop", c.optString("name"), sub = path?.let { "connected · $it" } ?: "not connected") { ping(c) }.tag = id
            val armed = confirm.armed == id
            button(if (armed) "Click again to unlink ${c.optString("name")}" else "Unlink", primary = false, small = true) {
                confirm.tap(id) { unlink(id) }
            }.tag = "unlink-$id"
        }
        if (loaded && computers.isEmpty()) note("No computers yet. On the computer, show its QR (the companion, or r1-link-pc pair).", size = 13f)
        msg?.let { note(it, size = 13f) }
    }

    private fun ping(c: JSONObject) {
        msg = "trying ${c.optString("name")}…"
        render()
        R1d.post("/api/link/ping", JSONObject().put("id", c.optString("id")).put("n", 5), onError = { msg = it; render() }) { r ->
            val rtts = r.optJSONArray("rtt_ms")
            val list = (0 until (rtts?.length() ?: 0)).map { rtts!!.optDouble(it) }
            msg = "${r.optString("computer")}: ${r.optJSONArray("paths")?.optString(0)}, " +
                "connected in ${r.optLong("connect_ms")} ms, round trips ${list.minOrNull()?.toInt()}–${list.maxOrNull()?.toInt()} ms"
            load()
        }
    }

    private fun unlink(id: String) = R1d.delete("/api/link/${R1d.enc(id)}", onError = { msg = it; render() }) { load() }

    override fun onDestroy() = confirm.cancel()
}

/**
 * Linking a computer (ADR-0048): the camera, facing out, reads the QR on the computer's screen. A frame goes to r1d
 * twice a second; it finds the QR and dials the computer by its key. Then this screen shows the four digits the
 * computer shows too, and waits for you to click **link** there (`link {state}`).
 */
class ScanScreen(shell: Shell) : Screen(shell) {
    private val finder = Viewfinder(context)
    private val guide = object : View(context) {
        private val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE; strokeWidth = context.dp(3f); color = Color.WHITE }
        override fun onDraw(c: Canvas) {
            val s = minOf(width, height) * 0.62f
            val l = (width - s) / 2
            val t = (height - s) / 2 - context.dp(20f)
            val k = s * 0.18f
            for ((x, sx) in listOf(l to 1f, l + s to -1f)) for ((y, sy) in listOf(t to 1f, t + s to -1f)) {
                c.drawLine(x, y, x + sx * k, y, p)
                c.drawLine(x, y, x, y + sy * k, p)
            }
        }
    }
    private val words = context.text(17f, 650, Color.WHITE).apply {
        gravity = Gravity.CENTER
        text = "point at the QR on your computer"
        setPadding(context.dpi(14f), context.dpi(8f), context.dpi(14f), context.dpi(8f))
        background = roundRect(Color.argb(160, 0, 0, 0), context.dp(16f))
    }
    private val result = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        gravity = Gravity.CENTER
        setBackgroundColor(Theme.BG)
        visibility = View.GONE
    }
    override val view = FrameLayout(context).apply {
        setBackgroundColor(Color.BLACK)
        addView(finder, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        addView(guide, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        addView(words, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL).apply {
            bottomMargin = context.dpi(28f)
        })
        addView(result, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }
    private var sending = false
    private var done = false
    private var shown = false
    private val look = object : Runnable {
        override fun run() {
            if (done) return
            send()
            view.postDelayed(this, LOOK_MS)
        }
    }

    init {
        finder.onError = { words.text = it }
        Motor.face(user = false)
    }

    /** A frame of the preview, small, to r1d: a QR in it pairs. One at a time. */
    private fun send() {
        if (sending || !shown) return
        val frame = finder.frame(FRAME_W, FRAME_H) ?: return
        val out = ByteArrayOutputStream()
        frame.compress(Bitmap.CompressFormat.JPEG, 85, out)
        frame.recycle()
        sending = true
        R1d.upload("/api/link/scan", out.toByteArray(), "image/jpeg", onError = { sending = false }) { r ->
            sending = false
            if (done || !r.optBoolean("found")) return@upload
            if (!r.optBoolean("link")) { words.text = "that's not a computer's code"; return@upload }
            val err = r.optString("error").takeIf { it.isNotEmpty() && it != "null" }
            if (err != null) { words.text = err; Sounds.play(Sounds.S.BUMP); return@upload }
            confirming(r.optJSONObject("paired") ?: return@upload)
        }
    }

    private val title = context.text(18f, 600, Theme.MUTED)
    private val name = context.text(28f, 750).apply { gravity = Gravity.CENTER }
    private val digits = context.text(64f, 700, Theme.accent).apply {
        typeface = Cam.MONO
        setPadding(0, context.dpi(18f), 0, context.dpi(6f))
    }
    private val hint = context.text(14f, 500, Theme.MUTED).apply {
        gravity = Gravity.CENTER
        setPadding(context.dpi(20f), 0, context.dpi(20f), 0)
    }

    /** Found and dialled: the code, and "click link on the computer". */
    private fun confirming(p: JSONObject) {
        done = true
        Sounds.play(Sounds.S.POP)
        finder.pause()
        result.removeAllViews()
        title.text = "confirm on"
        name.text = p.optString("name")
        digits.text = p.optString("code")
        hint.text = "check it shows the same code, and click link there"
        for (v in listOf(title, name, digits, hint)) result.addView(v)
        result.visibility = View.VISIBLE
    }

    /** The computer's answer. */
    override fun onAnyEvent(ev: Ev) {
        if (ev.type != "link" || result.visibility != View.VISIBLE) return
        when (ev.str("state")) {
            "paired" -> {
                Sounds.play(Sounds.S.POP)
                title.text = "linked to"
                hint.text = "manage your r1 from it now"
                view.postDelayed({ if (shell.top() === this) shell.back() }, 2500)
            }
            else -> {
                Sounds.play(Sounds.S.BUMP)
                title.text = "not linked"
                digits.setTextColor(Theme.MUTED)
                hint.text = ev.str("why") ?: "it didn't work"
                view.postDelayed({ if (shell.top() === this) shell.back() }, 3500)
            }
        }
    }

    override fun onEvent(ev: Ev): Boolean = ev.type == "wheel" || ev.isTap

    override fun onShow() {
        shown = true
        if (!done) {
            finder.resume()
            view.removeCallbacks(look)
            view.postDelayed(look, LOOK_MS)
        }
    }

    override fun onHide() {
        shown = false
        view.removeCallbacks(look)
        finder.pause()
    }

    override fun onDestroy() {
        done = true
        view.removeCallbacks(look)
        finder.pause()
        Motor.putBack()
    }

    private companion object {
        const val LOOK_MS = 500L
        const val FRAME_W = 600
        const val FRAME_H = 800
    }
}
