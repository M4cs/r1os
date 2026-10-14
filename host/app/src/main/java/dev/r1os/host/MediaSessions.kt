package dev.r1os.host

import android.app.SearchManager
import android.content.Context
import android.content.Intent
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import org.json.JSONObject

/**
 * Other apps' media (ADR-0040): what an Android app plays in the background (Spotify, a podcast app), so the glance
 * screen can show it and pause or stop it, as it does r1os's own [Player]. Reading other apps' sessions takes
 * MEDIA_CONTENT_CONTROL, a privileged permission r1os's image grants the host (from v0.2.12); without it there's
 * nothing here, and the glance shows r1os's own player only.
 *
 * Music by voice (R7): r1d hears what's playing ([report]) so System One knows "pause" and "next song" have something
 * to act on, and asks for what you said (`android_media`): pause, resume, skip, and play something a music app finds
 * ([playFromSearch]) through its media session, or its public "play from search" intent if it has none yet. Never by
 * working its screen.
 */
object MediaSessions {
    private const val TAG = "r1os"
    private val main = Handler(Looper.getMainLooper())

    /** What another app is playing, for the glance. */
    data class Now(val app: String, val title: String, val subtitle: String, val playing: Boolean, val pkg: String = "")

    var now: Now? = null
        private set
    private var controller: MediaController? = null
    private var onChange: () -> Unit = {}
    private var own = ""
    private var label: (String) -> String = { it }

    private val callback = object : MediaController.Callback() {
        override fun onPlaybackStateChanged(state: PlaybackState?) = pick()
        override fun onMetadataChanged(metadata: MediaMetadata?) = pick()
        override fun onSessionDestroyed() = pick()
    }
    private var controllers: List<MediaController> = emptyList()
    private var started = false

    /** Starts following the sessions; [onChange] runs on the main thread when what's shown changes. */
    fun start(context: Context, onChange: () -> Unit) {
        this.onChange = onChange
        // Once per process: a shell built again (a change of look) only takes the callback over.
        if (started) {
            main.post { onChange() }
            return
        }
        started = true
        own = context.packageName
        val pm = context.packageManager
        label = { pkg -> runCatching { pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString() }.getOrDefault(pkg) }
        val msm = context.getSystemService(MediaSessionManager::class.java) ?: return
        try {
            msm.addOnActiveSessionsChangedListener({ list -> follow(list.orEmpty()) }, null, main)
            follow(msm.getActiveSessions(null))
        } catch (e: SecurityException) {
            Log.i(TAG, "media sessions: not allowed (MEDIA_CONTENT_CONTROL comes with image v0.2.12)")
        }
    }

    private fun follow(list: List<MediaController>) {
        controllers.forEach { runCatching { it.unregisterCallback(callback) } }
        controllers = list.filter { it.packageName != own }
        controllers.forEach { it.registerCallback(callback, main) }
        pick()
    }

    /** The one to show: a playing one, else a paused one. */
    private fun pick() {
        fun st(c: MediaController) = c.playbackState?.state ?: PlaybackState.STATE_NONE
        val c = controllers.firstOrNull { st(it) == PlaybackState.STATE_PLAYING || st(it) == PlaybackState.STATE_BUFFERING }
            ?: controllers.firstOrNull { st(it) == PlaybackState.STATE_PAUSED }
        controller = c
        val next = c?.let {
            val m = it.metadata
            val title = m?.getString(MediaMetadata.METADATA_KEY_TITLE).orEmpty()
            val artist = m?.getString(MediaMetadata.METADATA_KEY_ARTIST) ?: m?.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST).orEmpty()
            Now(label(it.packageName), title.ifEmpty { label(it.packageName) }, artist, st(it) != PlaybackState.STATE_PAUSED, it.packageName)
        }
        if (next != now) {
            now = next
            onChange()
            report()
        }
    }

    /** r1d hears what's playing (on each change, and when it connects). */
    fun report() {
        val n = now
        val media = n?.let {
            JSONObject().put("package", it.pkg).put("app", it.app).put("title", it.title).put("artist", it.subtitle).put("playing", it.playing)
        }
        R1d.put("/api/android/media", JSONObject().put("media", media ?: JSONObject.NULL))
    }

    private fun of(pkg: String?): MediaController? = if (pkg.isNullOrEmpty()) controller else controllers.firstOrNull { it.packageName == pkg } ?: controller

    /** An app came to the front to play something: the shell brings r1os back once it's going. */
    var onLaunched: () -> Unit = {}

    /** r1d's asks (an agent's control_media or play_music). */
    fun onEvent(context: Context, ev: Ev) {
        val pkg = ev.str("package")
        when (ev.str("op")) {
            "play" -> {
                if (pkg == null) return
                val uri = ev.str("uri")
                val launched = if (uri != null) playLink(context, uri, pkg) else ev.str("query")?.let { playFromSearch(context, it, pkg) }
                if (launched == true) onLaunched()
            }
            "pause" -> of(pkg)?.transportControls?.pause()
            "resume" -> of(pkg)?.transportControls?.play()
            "toggle" -> toggle()
            "stop" -> of(pkg)?.transportControls?.stop()
            "next" -> of(pkg)?.transportControls?.skipToNext()
            "previous" -> of(pkg)?.transportControls?.skipToPrevious()
        }
    }

    fun pause() { controller?.transportControls?.pause() }
    fun play() { controller?.transportControls?.play() }
    fun next() { controller?.transportControls?.skipToNext() }
    fun previous() { controller?.transportControls?.skipToPrevious() }

    /** "Spotify: Song by Artist", or null when nothing's on. */
    fun describe(): String? = now?.let { n ->
        listOf(n.title, n.subtitle).filter { it.isNotBlank() }.joinToString(" by ").let { "${n.app}${if (n.playing) "" else " (paused)"}: $it" }
    }

    /**
     * A link that plays at once in the music app `pkg` (Spotify's own `spotify:…:play`, which r1d makes from its page),
     * delivered to the app as it's running. True when it came to the front.
     */
    fun playLink(context: Context, uri: String, pkg: String): Boolean {
        val i = Intent(Intent.ACTION_VIEW, android.net.Uri.parse(uri)).setPackage(pkg)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        return runCatching { context.startActivity(i); true }.getOrElse {
            Log.w(TAG, "music: $pkg can't open $uri", it)
            false
        }
    }

    /**
     * `query` played in the music app `pkg`: through its media session if it has one (it plays in the background), else
     * its "play from search" activity (as Google's assistant asks music apps), which brings it to the front. Null when
     * neither worked; true when the app came to the front.
     */
    fun playFromSearch(context: Context, query: String, pkg: String): Boolean? {
        val c = controllers.firstOrNull { it.packageName == pkg }
        if (c != null) {
            runCatching { c.transportControls.playFromSearch(query, Bundle()) }.onSuccess { return false }
        }
        val i = Intent(MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH).setPackage(pkg)
            .putExtra(SearchManager.QUERY, query)
            .putExtra(MediaStore.EXTRA_MEDIA_FOCUS, "vnd.android.cursor.item/*")
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        return runCatching { context.startActivity(i); true }.getOrElse {
            Log.w(TAG, "music: $pkg can't play from a search", it)
            null
        }
    }

    fun toggle() {
        val c = controller ?: return
        if (c.playbackState?.state == PlaybackState.STATE_PLAYING) c.transportControls.pause() else c.transportControls.play()
    }

    /** Most apps take stop as a pause that lets go of the session: it leaves the glance either way. */
    fun stop() {
        controller?.transportControls?.stop()
    }
}
