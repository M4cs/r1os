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
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject
import java.io.ByteArrayOutputStream

/**
 * Sharing by QR (ADR-0056, T-071): `what` (`{media: id}` or `{app: id}`) is served on the Wi-Fi while this shows, and
 * its URL is the QR. A phone's camera opens it. The screen stays on meanwhile; closing it (a click, or back) ends the
 * share, and so do 10 minutes.
 */
class ShareScreen(shell: Shell, private val what: JSONObject, label: String) : Screen(shell) {
    private val title = context.text(20f, 700).apply { text = label; gravity = Gravity.CENTER }
    private val frame = FrameLayout(context)
    private val hint = context.text(15f, 550, Theme.MUTED).apply {
        gravity = Gravity.CENTER
        text = "getting it ready…"
        setPadding(context.dpi(20f), 0, context.dpi(20f), 0)
    }
    override val view = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        gravity = Gravity.CENTER
        setBackgroundColor(Theme.BG)
        addView(title, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { bottomMargin = context.dpi(14f) })
        addView(frame, LinearLayout.LayoutParams(context.dpi(250f), context.dpi(250f)))
        addView(hint, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(16f) })
        keepScreenOn = true
    }
    private val ended = Runnable { end("the code has ended") }
    private var up = false

    init {
        R1d.post("/api/share", what, onError = { end(it) }) { r ->
            up = true
            frame.addView(QrView(context, r.optString("url")), FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
            hint.text = "scan it with your phone's camera\non the same wi-fi · for ${r.optLong("expires_s", 600) / 60} minutes"
            view.postDelayed(ended, r.optLong("expires_s", 600) * 1000)
        }
    }

    private fun end(why: String) {
        up = false
        view.removeCallbacks(ended)
        frame.removeAllViews()
        hint.text = why
        view.keepScreenOn = false
    }

    override fun onEvent(ev: Ev): Boolean {
        if (ev.isTap) {
            shell.back()
            return true
        }
        return ev.type == "wheel"
    }

    override fun onDestroy() {
        view.removeCallbacks(ended)
        if (up) R1d.delete("/api/share")
    }
}

/**
 * An app from another R1's share code (ADR-0056): the camera, facing out, reads the QR; r1d fetches the app from that
 * R1 and checks it; this shows its name and what it does, and a click adds it (the same checks as any app; its
 * permissions are asked for on the R1 as ever).
 */
class AppCodeScreen(shell: Shell) : Screen(shell) {
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
        text = "point at an app's code on another r1"
        setPadding(context.dpi(14f), context.dpi(8f), context.dpi(14f), context.dpi(8f))
        background = roundRect(Color.argb(160, 0, 0, 0), context.dp(16f))
    }
    private val result = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        gravity = Gravity.CENTER
        setBackgroundColor(Theme.BG)
        setPadding(context.dpi(24f), 0, context.dpi(24f), 0)
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
    private var shown = false
    /** The app found, waiting for a click: r1d's token for it. */
    private var token: String? = null
    private var adding = false
    private val look = object : Runnable {
        override fun run() {
            if (token != null) return
            send()
            view.postDelayed(this, LOOK_MS)
        }
    }

    init {
        finder.onError = { words.text = it }
        Motor.face(user = false)
    }

    /** A frame of the preview, small, to r1d: what its QR is. One at a time. */
    private fun send() {
        if (sending || !shown || token != null) return
        val frame = finder.frame(FRAME_W, FRAME_H) ?: return
        val out = ByteArrayOutputStream()
        frame.compress(Bitmap.CompressFormat.JPEG, 85, out)
        frame.recycle()
        sending = true
        R1d.upload("/api/scan", out.toByteArray(), "image/jpeg", onError = { sending = false }) { r ->
            sending = false
            if (token != null || !r.optBoolean("found")) return@upload
            val err = r.optString("error").takeIf { it.isNotEmpty() && it != "null" }
            words.text = when {
                err != null -> err
                r.optString("kind") == "link" -> "that's a computer's code: settings › computers"
                r.optString("kind") == "file" -> "that's a photo or a recording: open it on a phone"
                r.optString("kind") == "app" -> { found(r.optString("token"), r.optJSONObject("app") ?: JSONObject()); return@upload }
                else -> "that isn't an app's code"
            }
        }
    }

    private val name = context.text(28f, 750).apply { gravity = Gravity.CENTER }
    private val about = context.text(16f, 500, Theme.MUTED).apply { gravity = Gravity.CENTER; setPadding(0, context.dpi(6f), 0, context.dpi(18f)) }
    private val hint = context.text(15f, 600, Theme.accent).apply { gravity = Gravity.CENTER }

    /** Fetched and checked: what it is, and "click to add it". */
    private fun found(t: String, app: JSONObject) {
        token = t
        Sounds.play(Sounds.S.POP)
        finder.pause()
        val accent = Theme.COLORS[app.optString("accent")] ?: Theme.accent
        val tile = FrameLayout(context).apply {
            background = roundRect(Theme.withAlpha(accent, 0.2f), context.dp(18f))
            addView(android.widget.ImageView(context).apply { setImageDrawable(Icons.drawable(app.optString("icon", "app-window"), accent)) },
                FrameLayout.LayoutParams(context.dpi(34f), context.dpi(34f), Gravity.CENTER))
        }
        name.text = app.optString("name")
        about.text = app.optString("description").takeIf { it.isNotBlank() && it != "null" } ?: "an r1os app"
        hint.text = "click to add it · double click to leave it"
        result.removeAllViews()
        result.addView(tile, LinearLayout.LayoutParams(context.dpi(64f), context.dpi(64f)).apply { bottomMargin = context.dpi(14f) })
        for (v in listOf(name, about, hint)) result.addView(v)
        result.visibility = View.VISIBLE
    }

    private fun add() {
        val t = token ?: return
        if (adding) return
        adding = true
        hint.text = "adding…"
        R1d.post("/api/scan/install", JSONObject().put("token", t), onError = { adding = false; hint.text = it; Sounds.play(Sounds.S.BUMP) }) { r ->
            val id = r.optJSONObject("app")?.optString("id")
            Sounds.play(Sounds.S.POP)
            hint.text = "added"
            view.postDelayed({
                if (shell.top() !== this) return@postDelayed
                shell.back()
                if (!id.isNullOrEmpty()) shell.open(Target.of("app-info", "id" to id))
            }, 900)
        }
    }

    override fun onEvent(ev: Ev): Boolean {
        if (ev.isTap && token != null) {
            add()
            return true
        }
        return ev.type == "wheel" || ev.isTap
    }

    override fun onShow() {
        shown = true
        if (token == null) {
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
        token = token ?: ""
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
