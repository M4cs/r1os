package dev.r1os.host.shell

import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.Uri
import android.os.SystemClock
import android.text.TextUtils
import android.view.Gravity
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.keepCase
import dev.r1os.host.ui.text
import org.json.JSONObject
import java.util.Calendar
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

// The Recorder and the Journal as hardware (ADR-0037): a tape-to-tape deck, and a TP-7 for voice notes.

private val MATCH = FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT)

private fun JSONObject.list(key: String): List<JSONObject> {
    val a = optJSONArray(key) ?: return emptyList()
    return (0 until a.length()).mapNotNull { a.optJSONObject(it) }
}

/** A recording's player, read from r1d with the token. Plays, pauses, says where it is. */
private class Tape(private val screen: Screen, val item: JSONObject, private val onEnd: () -> Unit) {
    private var mp: MediaPlayer? = null
    private var ready = false
    var playing = false
        private set
    var position = 0L
    val duration get() = item.optLong("duration_ms").coerceAtLeast(1)
    /** Loudness, 20 a second (`/api/media/{id}/peaks`), for the meter. */
    var peaks = FloatArray(0)

    init {
        R1d.get("/api/media/${R1d.enc(item.optString("id"))}/peaks?per_s=20") { r ->
            val a = r.optJSONArray("peaks") ?: return@get
            peaks = FloatArray(a.length()) { a.optDouble(it).toFloat() }
        }
    }

    fun now(): Long = if (playing) mp?.currentPosition?.toLong() ?: position else position

    fun level(): Float = peaks.getOrNull((now() / 50).toInt()) ?: 0f

    fun play(onStart: () -> Unit) {
        val p = mp
        if (p != null && ready) {
            p.start()
            playing = true
            onStart()
            return
        }
        if (p != null) return
        val m = MediaPlayer()
        mp = m
        m.setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
        runCatching {
            m.setDataSource(screen.context, Uri.parse("${R1d.BASE}/api/media/${R1d.enc(item.optString("id"))}/file"), R1d.headers())
            m.setOnPreparedListener {
                ready = true
                if (position > 0) it.seekTo(position.toInt())
                it.start()
                playing = true
                onStart()
            }
            m.setOnCompletionListener {
                playing = false
                position = 0
                onEnd()
            }
            m.setOnErrorListener { _, _, _ -> release(); onEnd(); true }
            m.prepareAsync()
        }.onFailure { release(); onEnd() }
    }

    fun pause() {
        if (!playing) return
        position = mp?.currentPosition?.toLong() ?: position
        mp?.pause()
        playing = false
    }

    fun stop() {
        pause()
        position = 0
        if (ready) mp?.seekTo(0)
    }

    fun release() {
        runCatching { mp?.release() }
        mp = null
        ready = false
        playing = false
    }
}

// ---- the recorder: tape to tape ---------------------------------------------------------------------

/**
 * Tools › recorder (ADR-0033, ADR-0037): a tape-to-tape deck. Two reels with the tape between them, a
 * counter, a meter, and three keys (rec, play, stop) the wheel steps through; turn on past them for your
 * tapes. Recording goes on with the screen off (r1d keeps it); the reels only turn while you watch.
 *
 * - **rec** starts a tape, and stops it. **play** plays the loaded tape (the newest if none), and pauses.
 *   **stop** stops either, and winds the tape back.
 * - **A tape** in the list loads into the deck and plays. Its page (delete, the journal, its words) is the
 *   first row under the deck while it's loaded.
 * - **Hold to talk** with a tape loaded asks your agent about it.
 */
class RecorderScreen(shell: Shell) : Screen(shell) {
    private val accent = Theme.accent
    private val list = FocusList(context).apply {
        accent = this@RecorderScreen.accent
        setPadding(context.dpi(10f), 0, context.dpi(10f), context.dpi(20f))
        gap(6f)
    }
    override val view = FrameLayout(context).apply {
        setBackgroundColor(Theme.BG)
        addView(list, MATCH)
    }

    private val oled = Oled(context)
    private val state = GlyphView(context)
    private val counter = context.text(52f, 700).apply {
        fontFeatureSettings = "tnum"
        text = Deck.counter(0)
    }
    private val reels = Reels(context)
    private val words = context.text(14f, 500, Theme.MUTED).apply {
        keepCase()
        maxLines = 1
        ellipsize = TextUtils.TruncateAt.START
        gravity = Gravity.CENTER
    }
    private val meter = TickMeter(context)
    private val keys = TransportKeys(context).apply {
        accent = this@RecorderScreen.accent
        labels = listOf("rec", "play", "stop")
    }
    private val deck = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(context.dpi(4f), context.dpi(10f), context.dpi(4f), context.dpi(6f))
        addView(FrameLayout(context).apply {
            addView(context.text(26f, 500).apply { text = "recorder" }, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.START or Gravity.CENTER_VERTICAL))
            addView(oled, FrameLayout.LayoutParams(context.dpi(124f), context.dpi(50f), Gravity.END or Gravity.CENTER_VERTICAL))
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(52f)))
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(state, LinearLayout.LayoutParams(context.dpi(22f), context.dpi(22f)).apply { rightMargin = context.dpi(12f) })
            addView(counter)
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(8f) })
        addView(reels, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(176f)).apply { topMargin = context.dpi(4f) })
        addView(words, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
        addView(meter, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(14f)).apply { topMargin = context.dpi(10f) })
        addView(keys, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(74f)).apply { topMargin = context.dpi(12f) })
    }

    private var tapes: List<JSONObject> = emptyList()
    private var recId: String? = null
    /** `elapsedRealtime` at the recording's 0:00. */
    private var recBase = 0L
    private var live = ""
    private var tape: Tape? = null
    private var level = 0f
    /** The key the wheel is on; -1 while it's in the tapes. */
    private var keyFocus = 0
    private var shown = false
    private var clock = 0f
    private var msg: String? = null
    private val ticker = Ticker { frame(it) }
    /** Titles and summaries for each tape (T-068): null until r1d says. */
    private var summaries: Boolean? = null
    /** Marks on the tape being recorded, and the last one's words for a moment ("marked at 0:42"). */
    private var marks = 0
    private var marked: String? = null
    private val unmark = Runnable { marked = null; render() }

    init {
        rebuild()
        render()
    }

    private val recording get() = recId != null

    private fun load() {
        R1d.get("/api/recorder") { r ->
            val was = summaries
            summaries = r.optBoolean("summaries")
            if (was != summaries) rebuild()
            val now = r.optJSONObject("recording")
            if (now != null) {
                recId = now.optString("id")
                recBase = SystemClock.elapsedRealtime() - now.optLong("seconds") * 1000
            } else {
                recId = null
            }
            render()
            updateTicker()
        }
        R1d.get("/api/media?kind=recording&limit=200") { r ->
            tapes = r.list("media").filter { it.optString("id") != recId }
            rebuild()
            render()
        }
    }

    private fun number(t: JSONObject): Int = tapes.size - tapes.indexOfFirst { it.optString("id") == t.optString("id") }

    private fun labelOf(t: JSONObject): String =
        t.optString("label").ifEmpty { t.optString("transcript").take(48).ifEmpty { "tape %02d".format(number(t)) } }

    private fun rebuild(): Unit = list.rebuild {
        place(deck)
        tape?.let { t -> item("info", "this tape's page", sub = labelOf(t.item)) { shell.push(RecordingPage(shell, t.item)) } }
        if (tapes.isNotEmpty()) section("tapes")
        for (t in tapes) {
            val loaded = tape?.item?.optString("id") == t.optString("id")
            val at = t.optLong("at")
            item("cassette-tape", labelOf(t), accent = if (loaded) accent else null,
                sub = "${dayCaption(context, at)} · ${durationText(t.optLong("duration_ms"))}", trailing = "%02d".format(number(t))) {
                insert(t)
                keyFocus = 1
                list.blur()
                list.smoothScrollTo(0, 0)
                play()
            }
        }
        // An agent's title and summary for each tape when it stops (T-068): off unless you turn it on.
        summaries?.let { on ->
            section("")
            item("sparkle", "titles and summaries", if (on) accent else null, sub = "claude names each tape and sums it up when it stops",
                trailing = if (on) "on" else "off") {
                R1d.put("/api/recorder", JSONObject().put("summaries", !on), onError = { msg = it; render() }) { r -> summaries = r.optBoolean("summaries"); rebuild() }
            }.tag = "summaries"
        }
    }.also { if (keyFocus >= 0) list.blur() }

    /** Loads `t` into the deck (wound back). */
    private fun insert(t: JSONObject) {
        if (tape?.item?.optString("id") == t.optString("id")) return
        tape?.release()
        tape = Tape(this, t) { render(); updateTicker() }
        rebuild()
        render()
    }

    private fun render() {
        val t = tape
        val playing = t?.playing == true
        // Recording, the keys are mark, photo and stop (T-068); the counter's lamp says it's recording.
        keys.glyphs = if (recording) listOf(TransportKeys.Glyph.MARK, TransportKeys.Glyph.PHOTO, TransportKeys.Glyph.STOP)
        else listOf(TransportKeys.Glyph.REC, if (playing) TransportKeys.Glyph.PAUSE else TransportKeys.Glyph.PLAY, TransportKeys.Glyph.STOP)
        keys.labels = if (recording) listOf("mark", "photo", "stop") else listOf("rec", "play", "stop")
        keys.lit = if (playing && !recording) 1 else -1
        keys.focus = keyFocus
        reels.hot = recording
        meter.color = if (recording) Deck.REC else Theme.FG
        oled.hot = recording
        when {
            recording -> {
                oled.title = "rec"
                oled.line = if (marks > 0) "$marks mark${if (marks > 1) "s" else ""}" else "new tape"
                oled.box = "%02d".format(tapes.size + 1)
                state.glyph = TransportKeys.Glyph.REC
                state.color = Deck.REC
                words.text = marked ?: live.takeLast(80).ifEmpty { "listening…" }
            }
            t != null -> {
                oled.title = if (playing) "play" else if (t.position > 0) "pause" else "stop"
                oled.line = dayCaption(context, t.item.optLong("at")).take(10)
                oled.box = "%02d".format(number(t.item))
                state.glyph = if (playing) TransportKeys.Glyph.PLAY else if (t.position > 0) TransportKeys.Glyph.PAUSE else TransportKeys.Glyph.STOP
                state.color = if (playing) Theme.FG else Deck.DIM
                words.text = t.item.optString("transcript").ifEmpty { labelOf(t.item) }.take(90)
                counter.text = Deck.counter(t.now())
                reels.fraction = t.now().toFloat() / t.duration
            }
            else -> {
                oled.title = "ready"
                oled.line = if (tapes.isEmpty()) "no tapes" else "${tapes.size} tapes"
                oled.box = null
                state.glyph = TransportKeys.Glyph.STOP
                state.color = Deck.DIM
                words.text = msg ?: "turn to rec, click to record"
                counter.text = Deck.counter(0)
                reels.fraction = 0f
            }
        }
        if (!recording && !playing) meter.level = 0f
    }

    private fun frame(dt: Float) {
        clock += dt
        val t = tape
        when {
            recording -> {
                val ms = SystemClock.elapsedRealtime() - recBase
                counter.text = Deck.counter(ms)
                // A C-90's worth of tape winds across while it records.
                reels.fraction = ms / (90 * 60_000f)
                reels.advance(dt)
                level = max(0f, level - dt * 1.4f)
                meter.level = level
                keys.blinkOff = clock % 1f > 0.62f
            }
            t != null && t.playing -> {
                val pos = t.now()
                counter.text = Deck.counter(pos)
                reels.fraction = pos.toFloat() / t.duration
                reels.advance(dt)
                meter.level = t.level()
            }
            else -> ticker.stop()
        }
    }

    private fun updateTicker() {
        if ((recording || tape?.playing == true) && shown) ticker.start() else ticker.stop()
        if (!recording) keys.blinkOff = false
    }

    // ---- the keys -------------------------------------------------------------------------------------

    private fun press(key: Int) {
        Sounds.play(Sounds.S.POP)
        when (key) {
            0 -> if (recording) mark() else record()
            1 -> if (recording) photo() else play()
            2 -> if (recording) stopRecording() else { tape?.stop(); render(); updateTicker() }
        }
    }

    private fun record() {
        tape?.release()
        tape = null
        msg = null
        R1d.post("/api/recorder/start", onError = { msg = it; render() }) { r ->
            recId = r.optString("id")
            recBase = SystemClock.elapsedRealtime()
            live = ""
            marks = 0
            keyFocus = 0 // on mark: a click marks the moment
            Sounds.play(Sounds.S.LISTEN)
            rebuild()
            render()
            updateTicker()
        }
    }

    private fun stopRecording() = R1d.post("/api/recorder/stop", onError = { msg = it; render() })

    /** This moment of the tape, marked (T-068); with `photo`, the photo taken at it. */
    private fun mark(photo: String? = null) {
        val body = JSONObject().apply { photo?.let { put("photo", it) } }
        R1d.post("/api/recorder/mark", body, onError = { marked = it.lowercase(); render() }) { r ->
            val at = r.optJSONObject("mark")?.optLong("at_ms") ?: 0L
            marks++
            marked = "${if (photo != null) "photo" else "marked"} at ${Deck.counter(at).removePrefix("0:")}"
            view.removeCallbacks(unmark)
            view.postDelayed(unmark, 2500)
            render()
        }
    }

    /** The camera, for a photo of this moment: it's marked when you take it, and you're back here. */
    private fun photo() = shell.push(CameraScreen(shell, "recorder") { id, _ -> mark(id) })

    private fun play() {
        if (recording) return
        val t = tape ?: tapes.firstOrNull()?.let { insert(it); tape } ?: return
        if (t.playing) t.pause() else t.play { render(); updateTicker() }
        render()
        updateTicker()
    }

    // ---- input and events -----------------------------------------------------------------------------

    override fun onEvent(ev: Ev): Boolean {
        when {
            ev.type == "wheel" -> {
                val d = if (ev.int("delta") > 0) 1 else -1
                if (keyFocus >= 0) {
                    val next = keyFocus + d
                    when {
                        next in 0..2 -> { keyFocus = next; Sounds.play(Sounds.S.TICK) }
                        next > 2 && list.size > 0 -> { keyFocus = -1; list.setFocus(0); Sounds.play(Sounds.S.TICK) }
                        else -> Sounds.play(Sounds.S.BUMP)
                    }
                    keys.focus = keyFocus
                } else if (!list.move(d) && d < 0) {
                    keyFocus = 2
                    keys.focus = 2
                    list.blur()
                    list.smoothScrollTo(0, 0)
                }
                return true
            }
            ev.isTap -> {
                if (keyFocus >= 0) press(keyFocus) else list.activate()
                return true
            }
            // A double click stops a tape that's playing; otherwise it goes back.
            ev.isDoubleTap && tape?.playing == true -> {
                tape?.stop()
                render()
                updateTicker()
                return true
            }
            // Hold to talk about the loaded tape.
            ev.type == "voice" && ev.str("state") == "final" -> {
                val t = tape ?: return false
                val said = ev.str("text")?.trim().orEmpty()
                if (said.isEmpty()) return false
                shell.talkText("[About my recording `${t.item.optString("id")}` (read_recording has its words).] $said")
                return true
            }
        }
        return false
    }

    override fun onAnyEvent(ev: Ev) {
        when (ev.type) {
            "recording" -> if (ev.str("state") == "recording") {
                recId = ev.str("id")
                // Follow r1d's clock, but don't jitter the counter on every second.
                val base = SystemClock.elapsedRealtime() - ev.json.optLong("seconds") * 1000
                if (abs(base - recBase) > 1500) recBase = base
                live = ev.str("text").orEmpty()
                render()
                updateTicker()
            } else {
                val id = ev.str("id")
                recId = null
                live = ""
                updateTicker()
                // The tape just made goes into the deck, wound back, ready to play.
                R1d.get("/api/media?kind=recording&limit=200") { r ->
                    tapes = r.list("media")
                    tapes.firstOrNull { it.optString("id") == id }?.let { tape?.release(); tape = Tape(this, it) { render(); updateTicker() } }
                    keyFocus = 1
                    rebuild()
                    render()
                }
            }
            "recording_level" -> level = max(level, ev.num("level").toFloat())
            "media_changed" -> if (ev.str("kind") == "recording" && !recording) load()
        }
    }

    override fun onShow() {
        shown = true
        load()
        updateTicker()
    }

    override fun onHide() {
        shown = false
        tape?.pause()
        ticker.stop()
        render()
    }

    override fun onReconnect() = load()

    override fun onDestroy() {
        ticker.stop()
        tape?.release()
    }
}

// ---- the journal: a TP-7 for voice notes ------------------------------------------------------------

/**
 * Tools › journal (ADR-0033, ADR-0037), after teenage engineering's TP-7: a platter, a small display and
 * three keys. Your voice notes and the rest of the journal, newest first.
 *
 * - **Hold** to record a voice note: the platter spins and the lamp glows red while you talk, and letting
 *   go keeps it (r1d saves the audio and its words, `note` events).
 * - **The wheel** turns the platter a notch a note; the display shows the note's day, length and number.
 *   Past the oldest: **all entries**, the journal as a list (to delete, or for photos and typed notes).
 * - **A click** plays a voice note, and pauses it; a typed note or photo opens. **A double click** stops a
 *   note that's playing; otherwise it goes back.
 */
class JournalScreen(shell: Shell) : Screen(shell) {
    private val oled = Oled(context)
    private val platter = Platter(context)
    private val led = Led(context)
    private val caption = context.text(17f, 600).apply {
        keepCase()
        gravity = Gravity.CENTER
        maxLines = 2
        ellipsize = TextUtils.TruncateAt.END
    }
    private val meta = context.text(13f, 500, Theme.MUTED).apply {
        gravity = Gravity.CENTER
        maxLines = 1
    }
    private val keys = TransportKeys(context).apply { labels = listOf("hold", "click", "2×") }
    override val view = FrameLayout(context).apply {
        setBackgroundColor(Theme.BG)
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(context.dpi(14f), context.dpi(10f), context.dpi(14f), context.dpi(12f))
            addView(FrameLayout(context).apply {
                addView(context.text(26f, 500).apply { text = "journal" }, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.START or Gravity.CENTER_VERTICAL))
                addView(oled, FrameLayout.LayoutParams(context.dpi(124f), context.dpi(50f), Gravity.END or Gravity.CENTER_VERTICAL))
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(52f)))
            addView(FrameLayout(context).apply {
                addView(platter, MATCH)
                addView(led, FrameLayout.LayoutParams(context.dpi(28f), context.dpi(28f), Gravity.BOTTOM or Gravity.START))
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f).apply { topMargin = context.dpi(8f) })
            addView(caption, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(10f) })
            addView(meta, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(2f) })
            addView(keys, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, context.dpi(64f)).apply { topMargin = context.dpi(12f) })
        }, MATCH)
    }

    private var entries: List<JSONObject> = emptyList()
    private var loaded = false
    /** Which entry the platter is on; `entries.size` is "all entries". */
    private var sel = 0
    /** Keep the selection on this entry when the list comes again (a note just saved). */
    private var selId: String? = null
    private var recording = false
    private var recBase = 0L
    private var live = ""
    private var level = 0f
    private var tape: Tape? = null
    private var shown = false
    private var flash: String? = null
    /** The platter: where it is, and where it's going. */
    private var angle = 0f
    private var target = 0f
    private val ticker = Ticker { frame(it) }

    init {
        render()
    }

    private fun load(): Unit = R1d.get("/api/journal?limit=200") { r ->
        entries = r.list("entries")
        loaded = true
        val keep = selId ?: entries.getOrNull(sel)?.optString("id")
        sel = entries.indexOfFirst { it.optString("id") == keep }.takeIf { it >= 0 } ?: sel.coerceIn(0, entries.size)
        selId = null
        render()
    }

    private fun current(): JSONObject? = entries.getOrNull(sel)

    private fun isVoice(e: JSONObject?) = e?.optJSONObject("media_item")?.optString("kind") == "recording"

    /** Its number among its day's entries, the first of the day 1. */
    private fun ordinal(e: JSONObject): Int {
        val day = dayOf(e.optLong("at"))
        return entries.count { dayOf(it.optLong("at")) == day && it.optLong("at") <= e.optLong("at") }
    }

    private fun dayOf(ms: Long): Int = Calendar.getInstance().apply { timeInMillis = ms }.let { it.get(Calendar.YEAR) * 400 + it.get(Calendar.DAY_OF_YEAR) }

    private fun render() {
        val e = current()
        val playing = tape?.playing == true
        keys.glyphs = listOf(TransportKeys.Glyph.REC, if (playing) TransportKeys.Glyph.PAUSE else TransportKeys.Glyph.PLAY, TransportKeys.Glyph.STOP)
        keys.lit = when {
            recording -> 0
            playing -> 1
            else -> -1
        }
        oled.hot = recording
        led.color = when {
            recording -> Deck.REC
            playing -> Theme.FG
            else -> Deck.DIM
        }
        if (!recording) led.glow = if (playing) 0.25f else 0f
        meta.setTextColor(if (flash != null) Theme.BAD else Theme.MUTED)
        when {
            recording -> {
                oled.title = "rec"
                oled.line = Deck.dotted(SystemClock.elapsedRealtime() - recBase)
                oled.box = "%02d".format(entries.count { dayOf(it.optLong("at")) == dayOf(System.currentTimeMillis()) } + 1)
                caption.text = live.takeLast(120).ifEmpty { "listening…" }
                meta.text = "let go to keep it"
                platter.bottom = "rec"
            }
            !loaded -> {
                oled.title = "today"
                oled.line = Deck.dotted(0)
                oled.box = null
                caption.text = ""
                meta.text = ""
            }
            e == null && entries.isEmpty() -> {
                oled.title = "today"
                oled.line = Deck.dotted(0)
                oled.box = "00"
                caption.text = "hold to record a note"
                meta.text = flash ?: "your words are kept with it"
                platter.bottom = ""
            }
            e == null -> {
                oled.title = "all"
                oled.line = "${entries.size} entries"
                oled.box = null
                caption.text = "every entry, by day"
                meta.text = flash ?: "click to open"
                platter.bottom = "list"
            }
            else -> {
                val media = e.optJSONObject("media_item")
                oled.title = dayCaption(context, e.optLong("at")).take(9)
                oled.box = "%02d".format(ordinal(e))
                val at = Calendar.getInstance().apply { timeInMillis = e.optLong("at") }
                platter.bottom = clockText(context, at.get(Calendar.HOUR_OF_DAY) * 60 + at.get(Calendar.MINUTE))
                when {
                    isVoice(e) -> {
                        val t = tape?.takeIf { it.item.optString("id") == media?.optString("id") }
                        oled.line = Deck.dotted(t?.now()?.takeIf { it > 0 } ?: media?.optLong("duration_ms") ?: 0)
                        caption.text = e.optString("text").ifEmpty { media?.optString("transcript").orEmpty() }.ifEmpty { "no words heard" }
                        meta.text = flash ?: if (playing) "click to pause" else "click to play"
                    }
                    e.optString("kind") == "photo" -> {
                        oled.line = "photo"
                        caption.text = e.optString("text").ifEmpty { "a photo" }
                        meta.text = flash ?: "click to open"
                    }
                    else -> {
                        oled.line = "note"
                        caption.text = e.optString("text")
                        meta.text = flash ?: "click to open"
                    }
                }
            }
        }
    }

    private fun frame(dt: Float) {
        val spinning = recording || tape?.playing == true
        // About the TP-7's 33⅓ turns a minute.
        if (spinning) target += dt * 200f
        angle += (target - angle) * min(1f, dt * 14f)
        platter.angle = angle
        if (recording) {
            level = max(0f, level - dt * 1.6f)
            led.glow = 0.3f + 0.7f * level
            oled.line = Deck.dotted(SystemClock.elapsedRealtime() - recBase)
        } else if (tape?.playing == true) {
            oled.line = Deck.dotted(tape?.now() ?: 0)
            led.glow = 0.2f + 0.8f * (tape?.level() ?: 0f)
        }
        if (!spinning && abs(target - angle) < 0.1f) {
            angle = target
            platter.angle = angle
            ticker.stop()
        }
    }

    private fun wake() {
        if (shown) ticker.start()
    }

    private fun stopTape() {
        tape?.stop()
        render()
        wake()
    }

    override fun onEvent(ev: Ev): Boolean {
        when {
            ev.type == "wheel" -> {
                if (recording) return true
                val d = if (ev.int("delta") > 0) 1 else -1
                val next = (sel + d).coerceIn(0, entries.size)
                if (next == sel) {
                    Sounds.play(Sounds.S.BUMP)
                    return true
                }
                // Turning to another note stops the one playing, as scrubbing a tape would.
                if (tape?.playing == true) tape?.stop()
                sel = next
                flash = null
                target += d * 30f
                Sounds.play(Sounds.S.TICK)
                render()
                wake()
                return true
            }
            ev.isTap -> {
                if (recording) return true
                Sounds.play(Sounds.S.POP)
                val e = current()
                when {
                    e == null && entries.isNotEmpty() -> shell.push(JournalListPage(shell))
                    e == null -> {}
                    isVoice(e) -> {
                        val media = e.optJSONObject("media_item") ?: return true
                        if (tape?.item?.optString("id") != media.optString("id")) {
                            tape?.release()
                            tape = Tape(this, media) { render(); wake() }
                        }
                        val t = tape ?: return true
                        if (t.playing) t.pause() else t.play { render(); wake() }
                        render()
                        wake()
                    }
                    else -> shell.push(EntryPage(shell, e))
                }
                return true
            }
            ev.isDoubleTap && (tape?.playing == true || (tape?.position ?: 0) > 0) -> {
                stopTape()
                return true
            }
        }
        return false
    }

    override fun onAnyEvent(ev: Ev) {
        when (ev.type) {
            "note" -> when (ev.str("state")) {
                "listening" -> {
                    tape?.pause()
                    recording = true
                    recBase = SystemClock.elapsedRealtime()
                    live = ""
                    flash = null
                    Sounds.play(Sounds.S.LISTEN)
                    render()
                    wake()
                }
                "partial" -> {
                    live = ev.str("text").orEmpty()
                    if (recording) caption.text = live.takeLast(120)
                }
                "saved" -> {
                    recording = false
                    // The new note on the platter, ready to play.
                    selId = ev.str("entry")
                    Sounds.play(Sounds.S.NOTE)
                    load()
                }
                "cancelled" -> {
                    recording = false
                    render()
                }
                "error" -> {
                    recording = false
                    flash = ev.str("error") ?: "It wasn't kept."
                    render()
                }
            }
            "voice_level" -> if (recording) level = max(level, ev.num("level").toFloat())
            "journal_changed" -> if (!recording) load()
        }
    }

    override fun onShow() {
        shown = true
        // While this is on screen, a hold records a note (r1d's lease, ADR-0037).
        R1d.post("/api/journal/listen")
        load()
        wake()
    }

    override fun onHide() {
        shown = false
        R1d.delete("/api/journal/listen")
        tape?.pause()
        ticker.stop()
        render()
    }

    override fun onReconnect() {
        R1d.post("/api/journal/listen")
        load()
    }

    override fun onDestroy() {
        R1d.delete("/api/journal/listen")
        ticker.stop()
        tape?.release()
    }
}
