package dev.r1os.host.shell

import android.content.Context
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.Uri
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.camera.Photos
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import org.json.JSONObject
import java.util.Calendar
import java.util.Locale

// The R1's photos, recordings and journal (ADR-0031, ADR-0033): Tools › photos, recorder, journal.

/** Media items from r1d's list (`GET /api/media`). */
private fun JSONObject.items(key: String): List<JSONObject> {
    val a = optJSONArray(key) ?: return emptyList()
    return (0 until a.length()).mapNotNull { a.optJSONObject(it) }
}

/** "3:07", "1:02:45". */
fun durationText(ms: Long): String {
    val s = ms / 1000
    return if (s >= 3600) String.format(Locale.ROOT, "%d:%02d:%02d", s / 3600, s / 60 % 60, s % 60)
    else String.format(Locale.ROOT, "%d:%02d", s / 60, s % 60)
}

/** "today", "yesterday", "fri oct 9": a day's caption. */
fun dayCaption(context: Context, ms: Long): String {
    val c = Calendar.getInstance().apply { timeInMillis = ms }
    val today = Calendar.getInstance()
    val d = (today.get(Calendar.YEAR) * 400 + today.get(Calendar.DAY_OF_YEAR)) - (c.get(Calendar.YEAR) * 400 + c.get(Calendar.DAY_OF_YEAR))
    return when (d) {
        0 -> "today"
        1 -> "yesterday"
        else -> android.text.format.DateFormat.format("EEE MMM d", c).toString().lowercase()
    }
}

/** A photo's thumbnail as a wheel stop: padded, so the focus fill frames it. */
private fun FocusList.thumb(id: String, px: Int, onActivate: () -> Unit): ImageView {
    val v = ImageView(context).apply {
        scaleType = ImageView.ScaleType.CENTER_CROP
        val p = context.dpi(4f)
        setPadding(p, p, p, p)
        Photos.show(this, id, px)
    }
    register(v, onFocus = {}, onActivate = onActivate)
    return v
}

// ---- photos ---------------------------------------------------------------------------------------

/** Tools › photos: what the camera took, newest first, three to a row. A click opens one. */
class PhotosScreen(shell: Shell) : ListPage(shell, "Photos") {
    private var photos: List<JSONObject> = emptyList()
    private var loaded = false

    init {
        list.gap(4f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/media?kind=photo&limit=300") { r ->
        photos = r.items("media")
        loaded = true
        render()
    }

    override fun onShow() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "media_changed" && ev.str("kind") != "recording") load()
    }

    private fun render(): Unit = list.rebuild {
        if (!loaded) return@rebuild
        item("camera", "take a photo", accent) { shell.open(Target.of("camera")) }
        if (photos.isEmpty()) note("No photos yet.", center = true)
        val size = context.dpi(104f)
        for (row in photos.chunked(3)) {
            val line = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL }
            for (p in row) {
                val id = p.optString("id")
                line.addView(thumb(id, size) { shell.push(PhotoPage(shell, p)) }, LinearLayout.LayoutParams(size, size))
            }
            place(line)
        }
    }
}

/** One photo, big, with what you can do with it. Hold to talk asks your agent about it. */
class PhotoPage(shell: Shell, private val photo: JSONObject) : ListPage(shell, photo.optString("label").ifEmpty { "Photo" }) {
    private val photoId = photo.optString("id")
    private val confirm: Confirm = Confirm { render() }
    private var msg: String? = null

    init {
        list.gap(6f)
        render()
    }

    private fun render(): Unit = list.rebuild {
        val big = ImageView(context).apply {
            scaleType = ImageView.ScaleType.FIT_CENTER
            adjustViewBounds = true
            Photos.show(this, photoId, context.dpi(320f))
        }
        place(big)
        note(whenText(context, photo.optLong("at")) + " · hold the button to ask about it")
        msg?.let { note(it, Theme.FG) }
        item("notebook", "add to journal") {
            R1d.post("/api/journal", JSONObject().put("media", photoId).put("text", photo.optString("label")), onError = { msg = it; render() }) {
                msg = "In your journal."
                render()
            }
        }
        item("qr-code", "share by QR", sub = "to a phone on the same wi-fi") {
            shell.push(ShareScreen(shell, JSONObject().put("media", photoId), photo.optString("label").ifEmpty { "this photo" }))
        }
        val armed = confirm.armed == "delete"
        item("trash", if (armed) "Click again to delete" else "delete", danger = armed) {
            confirm.tap("delete") { R1d.delete("/api/media/${R1d.enc(photoId)}") { shell.back() } }
        }
    }

    override fun extra(ev: Ev): Boolean {
        if (ev.type != "voice" || ev.str("state") != "final") return false
        val t = ev.str("text")?.trim().orEmpty()
        if (t.isEmpty()) return false
        shell.talkAbout(photoId, t)
        return true
    }
}

// ---- recordings (the Recorder itself is in DeckScreens.kt) -----------------------------------------

/** One recording: play it, read its words, ask about it (hold to talk), keep it in the journal, delete it. */
class RecordingPage(shell: Shell, private val rec: JSONObject) : ListPage(shell, rec.optString("label").ifEmpty { "Recording" }) {
    private val recId = rec.optString("id")
    private val confirm: Confirm = Confirm { render() }
    private var player: MediaPlayer? = null
    private var playing = false
    private var msg: String? = null
    /** Where to start once the player's ready (a mark clicked before it was). */
    private var pendingSeek: Long? = null

    init {
        list.gap(6f)
        render()
    }

    private fun render(): Unit = list.rebuild {
        note("${whenText(context, rec.optLong("at"))} · ${durationText(rec.optLong("duration_ms"))}")
        item(if (playing) "pause" else "play", if (playing) "pause" else "play", accent) { toggle() }
        item("notebook", "add to journal") {
            R1d.post("/api/journal", JSONObject().put("media", recId).put("text", rec.optString("label")), onError = { msg = it; render() }) {
                msg = "In your journal."
                render()
            }
        }
        item("qr-code", "share by QR", sub = "to a phone on the same wi-fi") {
            stop()
            shell.push(ShareScreen(shell, JSONObject().put("media", recId), rec.optString("label").ifEmpty { "this recording" }))
        }
        msg?.let { note(it, Theme.FG) }
        val armed = confirm.armed == "delete"
        item("trash", if (armed) "Click again to delete" else "delete", danger = armed) {
            confirm.tap("delete") { stop(); R1d.delete("/api/media/${R1d.enc(recId)}") { shell.back() } }
        }
        // What an agent made of it, when you asked the Recorder for summaries (T-068).
        rec.optString("summary").takeIf { it.isNotBlank() }?.let { s ->
            section("summary")
            note(s, Theme.FG, 17f)
        }
        // The moments you marked as it recorded: a click plays from there.
        val marks = rec.optJSONArray("marks")
        if (marks != null && marks.length() > 0) section("marks")
        for (i in 0 until (marks?.length() ?: 0)) {
            val m = marks!!.optJSONObject(i) ?: continue
            val at = m.optLong("at_ms")
            val photo = m.optString("photo").isNotEmpty()
            item(if (photo) "camera" else "bookmark-simple", Deck.counter(at).removePrefix("0:"), sub = if (photo) "a photo, in your photos" else null) { playFrom(at) }
        }
        section("words")
        note(rec.optString("transcript").ifEmpty { "No words were recognised." }, Theme.FG, 17f)
    }

    private fun toggle() {
        val p = player
        if (p != null) {
            if (playing) p.pause() else p.start()
            playing = !playing
            render()
            return
        }
        val mp = MediaPlayer()
        player = mp
        mp.setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
        runCatching {
            mp.setDataSource(context, Uri.parse("${R1d.BASE}/api/media/${R1d.enc(recId)}/file"), R1d.headers())
            mp.setOnPreparedListener {
                pendingSeek?.let { ms -> it.seekTo(ms.toInt()) }
                pendingSeek = null
                it.start(); playing = true; render()
            }
            mp.setOnCompletionListener { playing = false; it.seekTo(0); render() }
            mp.setOnErrorListener { _, _, _ -> msg = "It won't play."; stop(); render(); true }
            mp.prepareAsync()
        }.onFailure { msg = "It won't play."; stop(); render() }
    }

    /** Plays from `ms` in (a mark). */
    private fun playFrom(ms: Long) {
        val p = player
        if (p == null) {
            pendingSeek = ms
            toggle()
            return
        }
        p.seekTo(ms.toInt())
        if (!playing) { p.start(); playing = true }
        render()
    }

    private fun stop() {
        runCatching { player?.release() }
        player = null
        playing = false
    }

    override fun onHide() {
        super.onHide()
        if (playing) { player?.pause(); playing = false; render() }
    }

    override fun onDestroy() = stop()

    /** Hold to talk: about this recording (the agent reads its transcript). */
    override fun extra(ev: Ev): Boolean {
        if (ev.type != "voice" || ev.str("state") != "final") return false
        val t = ev.str("text")?.trim().orEmpty()
        if (t.isEmpty()) return false
        shell.talkText("[About my recording `$recId` (read_recording has its words).] $t")
        return true
    }
}

// ---- journal --------------------------------------------------------------------------------------

/**
 * The journal as a list (Tools › journal, past the oldest note: "all entries"): notes, photos and recordings
 * by day, newest first, each opening its page (to delete it). Hold to talk and your words become an entry (no
 * agent needed); agents write in it too ("journal: …").
 */
class JournalListPage(shell: Shell) : ListPage(shell, "all entries") {
    private var entries: List<JSONObject> = emptyList()
    private var loaded = false
    private var msg: String? = null

    init {
        list.gap(6f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/journal?limit=200") { r ->
        entries = r.items("entries")
        loaded = true
        render()
    }

    override fun onShow() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "journal_changed") load()
    }

    private fun render(): Unit = list.rebuild {
        if (!loaded) return@rebuild
        msg?.let { note(it, Theme.FG) }
        if (entries.isEmpty()) note("Nothing written yet.", center = true)
        var day = ""
        for (e in entries) {
            val d = dayCaption(context, e.optLong("at"))
            if (d != day) { section(d); day = d }
            val time = clockText(context, Calendar.getInstance().apply { timeInMillis = e.optLong("at") }.let { it.get(Calendar.HOUR_OF_DAY) * 60 + it.get(Calendar.MINUTE) })
            val media = e.optJSONObject("media_item")
            when (e.optString("kind")) {
                "photo" -> {
                    val thumb = ImageView(context).apply {
                        scaleType = ImageView.ScaleType.CENTER_CROP
                        background = roundRect(Theme.CARD, context.dp(10f))
                        clipToOutline = true
                        media?.optString("id")?.let { Photos.show(this, it, context.dpi(48f)) }
                    }
                    item("image", e.optString("text").ifEmpty { "photo" }, sub = time, trailingView = thumb.also {
                        it.layoutParams = LinearLayout.LayoutParams(context.dpi(48f), context.dpi(48f))
                    }) { shell.push(EntryPage(shell, e)) }
                }
                "recording" -> item("waveform", e.optString("text").ifEmpty { media?.optString("label")?.ifEmpty { null } ?: "recording" },
                    sub = "$time · ${durationText(media?.optLong("duration_ms") ?: 0)}") { shell.push(EntryPage(shell, e)) }
                else -> item("note-pencil", e.optString("text"), bold = false, sub = time) { shell.push(EntryPage(shell, e)) }
            }
        }
    }

    /** Hold to talk: the words go in as an entry. */
    override fun extra(ev: Ev): Boolean {
        if (ev.type != "voice" || ev.str("state") != "final") return false
        val t = ev.str("text")?.trim().orEmpty()
        if (t.isEmpty()) return false
        R1d.post("/api/journal", JSONObject().put("text", t), onError = { msg = it; render() }) { msg = null; Sounds.play(Sounds.S.NOTE) }
        return true
    }
}

/** One journal entry: its words, its photo or recording, and deleting it. */
class EntryPage(shell: Shell, private val entry: JSONObject) : ListPage(shell, dayCaption(shell.context, entry.optLong("at"))) {
    private val confirm: Confirm = Confirm { render() }

    init {
        list.gap(6f)
        render()
    }

    private fun render(): Unit = list.rebuild {
        val media = entry.optJSONObject("media_item")
        if (media != null && media.optString("kind") == "photo") {
            place(ImageView(context).apply {
                scaleType = ImageView.ScaleType.FIT_CENTER
                adjustViewBounds = true
                Photos.show(this, media.optString("id"), context.dpi(320f))
            })
        }
        entry.optString("text").takeIf { it.isNotBlank() }?.let { note(it, Theme.FG, 19f) }
        note(whenText(context, entry.optLong("at")))
        if (media != null && media.optString("kind") == "recording") {
            item("waveform", "open the recording", accent) { shell.push(RecordingPage(shell, media)) }
        }
        val armed = confirm.armed == "delete"
        item("trash", if (armed) "Click again to delete" else "delete from the journal", danger = armed) {
            confirm.tap("delete") { R1d.delete("/api/journal/${R1d.enc(entry.optString("id"))}") { shell.back() } }
        }
    }
}
