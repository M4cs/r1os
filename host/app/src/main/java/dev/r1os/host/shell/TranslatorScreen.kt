package dev.r1os.host.shell

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.Uri
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin

/**
 * Tools › translator (ADR-0034, ADR-0037): two channels, you and them, like a two-track mixer.
 *
 * Hold the button and speak either language: r1d works out which (push-to-talk's recognizer for English,
 * live; Whisper for the other), and the arrow between the channels shows the turn's direction. The
 * translation shows below in large type and plays in the other channel's voice.
 *
 * - **The wheel** steps through the two channels and the keys; **a click** on a channel picks its language,
 *   on **again** says the last translation again, on **stop** stops it. **A double click** goes back.
 * - **Getting ready** (r1d's `translator_stage` events, no polling): the meters sweep while the models
 *   load, the English channel settles first (it's heard at once), then both fill once and come to rest.
 *
 * While this is on screen, push-to-talk is the Translator's (r1d's lease: `POST /api/translator` on show,
 * `DELETE` on hide).
 */
class TranslatorScreen(shell: Shell, askedMine: String? = null, askedTheirs: String? = null) : Screen(shell) {
    private val accent = Theme.colorOf("cyan")
    private val prefs = context.getSharedPreferences("translator", Context.MODE_PRIVATE)
    // The languages asked for (an agent's open_screen, System One's "translate English to Portuguese") stay chosen.
    private var mine = (askedMine ?: prefs.getString("mine", "en") ?: "en").also { if (askedMine != null) prefs.edit().putString("mine", it).apply() }
    private var theirs = (askedTheirs ?: prefs.getString("theirs", "es") ?: "es").also { if (askedTheirs != null) prefs.edit().putString("theirs", it).apply() }
    private var languages: List<JSONObject> = emptyList()

    private val oled = Oled(context)
    private val you = Channel(context, "you")
    private val them = Channel(context, "them")
    private val arrow = Arrow(context)
    private val said = context.text(23f, 700).apply {
        keepCase()
        maxLines = 4
        ellipsize = TextUtils.TruncateAt.END
    }
    private val heardLine = context.text(14f, 500, Theme.MUTED).apply {
        keepCase()
        maxLines = 2
        ellipsize = TextUtils.TruncateAt.END
    }
    private val keys = TransportKeys(context).apply {
        accent = this@TranslatorScreen.accent
        labels = listOf("hold", "again", "stop")
    }
    override val view = FrameLayout(context).apply {
        setBackgroundColor(Theme.BG)
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(context.dpi(14f), context.dpi(10f), context.dpi(14f), context.dpi(12f))
            addView(FrameLayout(context).apply {
                addView(context.text(26f, 500).apply { text = "translator" }, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.START or Gravity.CENTER_VERTICAL))
                addView(oled, FrameLayout.LayoutParams(context.dpi(124f), context.dpi(50f), Gravity.END or Gravity.CENTER_VERTICAL))
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(52f)))
            addView(FrameLayout(context).apply {
                addView(LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    addView(you, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f).apply { rightMargin = context.dpi(5f) })
                    addView(them, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f).apply { leftMargin = context.dpi(5f) })
                }, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
                addView(arrow, FrameLayout.LayoutParams(context.dpi(28f), context.dpi(28f), Gravity.CENTER))
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(142f)).apply { topMargin = context.dpi(10f) })
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(context.dpi(4f), 0, context.dpi(4f), 0)
                addView(said)
                addView(heardLine, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(6f) })
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
            addView(keys, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(64f)).apply { topMargin = context.dpi(8f) })
        }, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }

    /** off · loading · english · ready (r1d), or `missing` (no models). */
    private var stage = "off"
    private var shown = false
    /** 0 you, 1 them, 2 again, 3 stop. */
    private var focus = 1
    /** The turn: listening · hearing · heard · translated, or null between turns. */
    private var turn: String? = null
    private var from: String? = null
    private var to: String? = null
    private var level = 0f
    private var lastSpeech = 0L
    private var turns = 0
    /** Something has been said or shown since the screen opened (the hint stops showing). */
    private var spoken = false
    private var player: MediaPlayer? = null
    private var playing = false
    private var clock = 0f
    /** The "ready" flourish: both meters full, falling away. */
    private var flourish = 0f
    private val ticker = Ticker { frame(it) }

    init {
        render()
    }

    // ---- r1d ---------------------------------------------------------------------------------------------

    private fun start() {
        R1d.post("/api/translator", JSONObject().put("mine", mine).put("theirs", theirs), onError = { message(it, bad = true) }) { r ->
            r.optJSONArray("languages")?.let { a -> languages = (0 until a.length()).mapNotNull { a.optJSONObject(it) } }
            stage = if (!r.optBoolean("installed", true)) "missing" else r.optString("stage", "loading")
            render()
            wake()
        }
    }

    private fun name(code: String): String = languages.firstOrNull { it.optString("code") == code }?.optString("name")?.lowercase() ?: code

    /** Ready to hear `code`: English once the stage is past loading, the other language when it's ready. */
    private fun ready(code: String): Boolean = when (stage) {
        "ready" -> true
        "english" -> code == "en"
        else -> false
    }

    // ---- drawing -----------------------------------------------------------------------------------------

    private fun render() {
        you.code = mine
        you.label = name(mine)
        them.code = theirs
        them.label = name(theirs)
        you.focused = focus == 0
        them.focused = focus == 1
        you.accent = accent
        them.accent = accent
        you.ready = ready(mine)
        them.ready = ready(theirs)
        keys.focus = when (focus) {
            2 -> 1
            3 -> 2
            else -> -1
        }
        keys.lit = when {
            turn == "listening" -> 0
            playing -> 1
            else -> -1
        }
        // The lamps: red on the channel speaking, white on the one being spoken for.
        you.lamp = lampFor(mine)
        them.lamp = lampFor(theirs)
        arrow.dir = when {
            from == null || to == null -> 0
            from == mine -> 1
            else -> -1
        }
        arrow.lit = turn != null || playing
        oled.hot = turn == "listening"
        oled.title = when {
            stage == "missing" -> "none"
            stage == "loading" || stage == "off" -> "load"
            stage == "english" && turn == null && !playing -> "load"
            turn == "listening" -> "listen"
            turn == "hearing" -> "hear"
            turn == "heard" -> "turn"
            turn == "translated" || playing -> "say"
            else -> "ready"
        }
        oled.line = when {
            from != null && to != null -> "$from → $to"
            stage == "english" -> "en ready"
            else -> "$mine ⇄ $theirs"
        }
        oled.box = if (turns > 0) "%02d".format(turns) else null
        if (!spoken) {
            when (stage) {
                "missing" -> hint("The Translator's models aren't on the R1 yet (tools/fetch-models.sh, then deploy).", bad = true)
                "loading", "off" -> hint("getting ready…")
                "english" -> hint(if (mine == "en" || theirs == "en") "english is ready; ${name(if (mine == "en") theirs else mine)} in a moment" else "getting ready…")
                else -> hint("hold the button and speak either language")
            }
        }
    }

    private fun lampFor(code: String): Int = when {
        turn == "listening" && from == null -> Theme.withAlpha(Deck.REC, 0.45f)
        turn != null && from == code -> Deck.REC
        playing && to == code -> Theme.FG
        else -> Deck.DIM
    }

    /** The hint before anything's said: smaller and muted. */
    private fun hint(text: String, bad: Boolean = false) {
        said.text = text
        said.setTextColor(if (bad) Theme.BAD else Theme.MUTED)
        said.textSize = 17f
        heardLine.text = ""
    }

    /** A turn's words in the big type. */
    private fun words(text: String, quiet: Boolean = false) {
        spoken = true
        said.text = text
        said.setTextColor(if (quiet) Theme.MUTED else Theme.FG)
        said.textSize = if (quiet) 17f else 23f
    }

    private fun message(text: String, bad: Boolean = false) {
        spoken = true
        said.text = text
        said.setTextColor(if (bad) Theme.BAD else Theme.FG)
        said.textSize = 17f
        heardLine.text = ""
    }

    private fun frame(dt: Float) {
        clock += dt
        val loading = stage == "loading" || stage == "off" || stage == "english"
        // The boot sweep, on the channels not ready yet.
        you.sweep = if (loading && !you.ready) clock else null
        them.sweep = if (loading && !them.ready) clock else null
        when {
            flourish > 0f -> {
                flourish = max(0f, flourish - dt)
                val f = flourish / FLOURISH
                you.level = f
                them.level = f
            }
            turn == "listening" -> {
                level = max(0f, level - dt * 1.6f)
                you.level = if (from == null || from == mine) level else 0f
                them.level = if (from == null || from == theirs) level else 0f
            }
            playing -> {
                // The voice: a gentle swell on the channel it speaks for.
                val l = 0.4f + 0.3f * sin(clock * 9f) * sin(clock * 2.3f)
                you.level = if (to == mine) l else 0f
                them.level = if (to == theirs) l else 0f
            }
            else -> {
                you.level = 0f
                them.level = 0f
            }
        }
        if (!loading && flourish <= 0f && turn != "listening" && !playing) ticker.stop()
    }

    private fun wake() {
        if (shown) ticker.start()
    }

    // ---- turns ------------------------------------------------------------------------------------------

    private fun play(speech: Long) {
        stopSpeech()
        val mp = MediaPlayer()
        player = mp
        mp.setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
        runCatching {
            mp.setDataSource(context, Uri.parse("${R1d.BASE}/api/translator/speech/$speech"), R1d.headers())
            mp.setOnPreparedListener { it.start(); playing = true; render(); wake() }
            mp.setOnCompletionListener { stopSpeech() }
            mp.setOnErrorListener { _, _, _ -> stopSpeech(); true }
            mp.prepareAsync()
        }.onFailure { stopSpeech() }
    }

    private fun stopSpeech() {
        runCatching { player?.release() }
        player = null
        if (playing) {
            playing = false
            render()
        }
    }

    override fun onAnyEvent(ev: Ev) {
        when (ev.type) {
            "translator_stage" -> {
                val was = stage
                stage = ev.str("stage") ?: stage
                if (stage == "ready" && was != "ready" && shown) {
                    flourish = FLOURISH
                    Sounds.play(Sounds.S.POP)
                }
                render()
                wake()
            }
            "translation" -> {
                when (val state = ev.str("state")) {
                    "listening" -> {
                        stopSpeech()
                        turn = state
                        from = null
                        to = null
                        level = 0f
                        heardLine.text = ""
                        words("listening…", quiet = true)
                    }
                    // English, live (push-to-talk's recognizer).
                    "partial" -> {
                        from = "en".takeIf { it == mine || it == theirs }
                        words(ev.str("heard").orEmpty())
                    }
                    "hearing" -> {
                        turn = state
                        if (from == null) words("…", quiet = true)
                    }
                    "heard" -> {
                        turn = state
                        from = ev.str("from")
                        to = ev.str("to")
                        words("…", quiet = true)
                        heardLine.text = "${name(from.orEmpty())}: ${ev.str("heard").orEmpty()}"
                    }
                    "translated", "spoken" -> {
                        if (state == "translated") turns++
                        turn = if (state == "spoken") null else state
                        from = ev.str("from") ?: from
                        to = ev.str("to") ?: to
                        words(ev.str("text").orEmpty())
                        heardLine.text = "${name(from.orEmpty())}: ${ev.str("heard").orEmpty()}" + if (ev.str("via") == "agent") " · via the agent" else ""
                        val speech = ev.json.optLong("speech", 0)
                        if (state == "spoken" && speech > 0) {
                            lastSpeech = speech
                            play(speech)
                        }
                    }
                    "error" -> {
                        turn = null
                        message(ev.str("error") ?: "Something went wrong.", bad = true)
                    }
                    "cancelled" -> {
                        turn = null
                        spoken = false
                    }
                }
                render()
                wake()
            }
            "voice_level" -> if (turn == "listening") level = max(level, ev.num("level").toFloat())
        }
    }

    // ---- input ------------------------------------------------------------------------------------------

    override fun onEvent(ev: Ev): Boolean {
        when {
            ev.type == "wheel" -> {
                val next = focus + if (ev.int("delta") > 0) 1 else -1
                if (next in 0..3) {
                    focus = next
                    Sounds.play(Sounds.S.TICK)
                    render()
                } else {
                    Sounds.play(Sounds.S.BUMP)
                }
                return true
            }
            ev.isTap -> {
                Sounds.play(Sounds.S.POP)
                when (focus) {
                    0, 1 -> pick(focus == 0)
                    2 -> if (lastSpeech > 0) play(lastSpeech)
                    3 -> stopSpeech()
                }
                return true
            }
            ev.isDoubleTap && playing -> {
                stopSpeech()
                return true
            }
        }
        return false
    }

    private fun pick(yours: Boolean) {
        if (languages.isEmpty()) return
        val current = if (yours) mine else theirs
        val other = if (yours) theirs else mine
        shell.push(LanguagePage(shell, if (yours) "you speak" else "they speak", languages, current, other) { code ->
            if (yours) mine = code else theirs = code
            prefs.edit().putString("mine", mine).putString("theirs", theirs).apply()
            turns = 0
            spoken = false
            from = null
            to = null
            render()
        })
    }

    override fun onShow() {
        shown = true
        start()
        wake()
    }

    override fun onHide() {
        shown = false
        ticker.stop()
        stopSpeech()
        R1d.delete("/api/translator")
    }

    /** r1d restarted (a deploy): the lease went with it. Only while on screen. */
    override fun onReconnect() {
        if (shown) start()
    }

    override fun onDestroy() {
        ticker.stop()
        stopSpeech()
        R1d.delete("/api/translator")
    }

    private companion object {
        const val FLOURISH = 0.6f
    }
}

/** A language to pick for one side: each says whether it's translated on the R1 and spoken. */
private class LanguagePage(
    shell: Shell,
    title: String,
    languages: List<JSONObject>,
    current: String,
    other: String,
    onPick: (String) -> Unit,
) : ListPage(shell, title, Theme.colorOf("cyan")) {
    init {
        list.gap(6f)
        val shown = languages.filter { it.optString("code") != other }
        list.rebuild {
            for (l in shown) {
                val code = l.optString("code")
                val onDevice = l.optJSONArray("on_device")?.let { a -> (0 until a.length()).any { a.optString(it) == other } } ?: false
                val sub = listOfNotNull(if (onDevice) "on the R1" else "through the agent", if (l.optBoolean("voice")) "spoken" else null).joinToString(" · ")
                item(if (code == current) "check" else "globe", l.optString("name").lowercase(), sub = sub, trailing = code) {
                    onPick(code)
                    shell.back()
                }
            }
        }
        val at = shown.indexOfFirst { it.optString("code") == current }
        if (at >= 0) list.post { list.setFocus(at, animate = false) }
    }
}

/**
 * One side of the conversation: who (you, them), the language's code large and its name, a lamp, and a
 * strip of ticks for the level. Focused, it fills with the accent. Until it's ready its code is dim and
 * its ticks sweep ([sweep]: the boot animation's clock).
 */
private class Channel(context: Context, private val who: String) : View(context) {
    var code = ""
        set(v) { if (field != v) { field = v; invalidate() } }
    var label = ""
        set(v) { if (field != v) { field = v; invalidate() } }
    var focused = false
        set(v) { if (field != v) { field = v; invalidate() } }
    var ready = false
        set(v) { if (field != v) { field = v; invalidate() } }
    var lamp = Deck.DIM
        set(v) { if (field != v) { field = v; invalidate() } }
    var level = 0f
        set(v) { if (abs(field - v) > 0.01f) { field = v; invalidate() } }
    var sweep: Float? = null
        set(v) { if (field != v) { field = v; invalidate() } }
    var accent = Theme.colorOf("cyan")

    private val p = Paint(Paint.ANTI_ALIAS_FLAG)
    private val big = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Theme.font(700); textSize = context.dp(46f) }
    private val small = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Theme.font(600); textSize = context.dp(12f) }
    private val name = android.text.TextPaint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Theme.font(500); textSize = context.dp(14f) }
    private val r = RectF()

    override fun onDraw(c: Canvas) {
        val radius = context.dp(14f)
        val pad = context.dp(12f)
        r.set(0f, 0f, width.toFloat(), height.toFloat())
        p.color = if (focused) accent else Deck.KEY
        c.drawRoundRect(r, radius, radius, p)
        if (!focused) {
            p.color = Deck.KEY_TOP
            c.drawRect(radius, 0f, width - radius, context.dp(1f), p)
        }
        val ink = if (focused) Theme.ON_ACCENT else Theme.FG
        val quiet = if (focused) Theme.withAlpha(Theme.ON_ACCENT, 0.65f) else Theme.MUTED
        small.color = quiet
        c.drawText(who, pad, pad + small.textSize * 0.8f, small)
        // The lamp.
        p.color = if (focused && lamp == Deck.DIM) Theme.withAlpha(Theme.ON_ACCENT, 0.3f) else lamp
        c.drawCircle(width - pad - context.dp(4f), pad + context.dp(4f), context.dp(4.5f), p)
        // The code, dim until it's ready.
        big.color = if (ready) ink else Theme.withAlpha(ink, 0.35f)
        val codeY = height * 0.58f
        // Clear of the arrow between the channels.
        val inset = pad + context.dp(8f)
        c.drawText(code.lowercase(), inset - context.dp(2f), codeY, big)
        name.color = quiet
        c.drawText(TextUtils.ellipsize(label, name, width - inset - pad, TextUtils.TruncateAt.END).toString(), inset, codeY + context.dp(20f), name)
        // The ticks: the level, or the sweep while it gets ready.
        val tw = context.dp(2f)
        val y1 = height - pad
        val y0 = y1 - context.dp(10f)
        val step = (width - pad * 2 - tw) / (TICKS - 1)
        val lit = (level * TICKS).toInt()
        val s = sweep
        val head = if (s != null) (sin(s * 3.2f) + 1f) / 2f * (TICKS - 1) else -10f
        for (i in 0 until TICKS) {
            val on = i < lit || (s != null && abs(i - head) < 1.6f)
            p.color = when {
                on && focused -> Theme.ON_ACCENT
                on -> if (s != null) accent else Theme.FG
                focused -> Theme.withAlpha(Theme.ON_ACCENT, 0.25f)
                else -> Deck.DIM
            }
            val x = pad + i * step
            c.drawRoundRect(x, y0, x + tw, y1, tw, tw, p)
        }
    }

    private companion object {
        const val TICKS = 14
    }
}

/** Between the channels: which way the turn goes (→ you to them, ← them to you), lit while it does. */
private class Arrow(context: Context) : View(context) {
    var dir = 0
        set(v) { if (field != v) { field = v; invalidate() } }
    var lit = false
        set(v) { if (field != v) { field = v; invalidate() } }
    private val p = Paint(Paint.ANTI_ALIAS_FLAG)
    private val path = Path()

    override fun onDraw(c: Canvas) {
        val cx = width / 2f
        val cy = height / 2f
        val r = min(width, height) / 2f - context.dp(1f)
        p.style = Paint.Style.FILL
        p.color = android.graphics.Color.BLACK
        c.drawCircle(cx, cy, r, p)
        p.style = Paint.Style.STROKE
        p.strokeWidth = context.dp(1.2f)
        p.color = Deck.HAIR
        c.drawCircle(cx, cy, r, p)
        p.style = Paint.Style.FILL
        p.color = if (lit) Theme.FG else Deck.DIM
        val s = r * 0.42f
        path.reset()
        if (dir == 0) {
            // At rest: two small heads, one each way.
            path.moveTo(cx - s, cy - s * 0.45f); path.lineTo(cx - s * 0.1f, cy - s * 0.95f); path.lineTo(cx - s * 0.1f, cy + s * 0.05f); path.close()
            path.moveTo(cx + s, cy + s * 0.45f); path.lineTo(cx + s * 0.1f, cy + s * 0.95f); path.lineTo(cx + s * 0.1f, cy - s * 0.05f); path.close()
        } else {
            val d = dir.toFloat()
            path.moveTo(cx + d * s, cy)
            path.lineTo(cx - d * s * 0.6f, cy - s * 0.8f)
            path.lineTo(cx - d * s * 0.6f, cy + s * 0.8f)
            path.close()
        }
        c.drawPath(path, p)
    }
}
