package dev.r1os.host

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.MediaPlayer
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.util.Log
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import org.json.JSONObject
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.Executors

/**
 * The R1's player for apps (ADR-0040): what an app's `play` step starts (r1d checked it, crate::player) plays on after
 * the user leaves the app, until it's paused or stopped: from the app, the glance screen's controls, an agent, or
 * "pause" said to System One.
 *
 * - **MediaPlayer** opens what r1d says: its relay on loopback (with the host's token: the only cleartext the host
 *   speaks), or an HLS stream directly.
 * - **[PlayerService]** keeps it playing in the background: a foreground service of the media-playback type, with the
 *   CPU and Wi-Fi kept awake while it plays (and only then).
 * - **A MediaSession**, so a headset's buttons work, and **audio focus**: it pauses for a call or another player, and
 *   dips while you hold the button to talk ([duck]).
 * - **r1d hears what it's doing** (`PUT /api/player`): the app's page shows it (`/player`), and so does the glance.
 */
object Player {
    private const val TAG = "r1os"
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()

    /** What's playing, for the glance screen. */
    data class Now(val app: String, val state: String, val title: String, val subtitle: String, val song: String)

    var now = Now("", "stopped", "", "", "")
        private set
    private val listeners = CopyOnWriteArraySet<(Now) -> Unit>()

    fun onChange(fn: (Now) -> Unit): () -> Unit {
        listeners.add(fn)
        return { listeners.remove(fn) }
    }

    private var app: Context? = null
    private var mp: MediaPlayer? = null
    /** [mp] is ready to start (a stream paused while it loaded starts on resume, not on its own). */
    private var prepared = false
    private var session: MediaSession? = null
    private var focus: AudioFocusRequest? = null
    private var wifi: WifiManager.WifiLock? = null
    /** Paused for a moment by audio focus (a call, a notification), to resume when it comes back. */
    private var pausedForFocus = false
    private var ducked = false
    private var dipped = false

    val active get() = now.state != "stopped"

    // ---- what r1d asks (`media` events) ------------------------------------------------------------

    fun onEvent(context: Context, ev: Ev) {
        when (ev.str("op")) {
            "play" -> play(context, ev.str("app").orEmpty(), ev.str("url") ?: return, ev.str("title").orEmpty(), ev.str("subtitle").orEmpty())
            "pause" -> pause()
            "resume" -> resume()
            "toggle" -> toggle()
            "stop" -> stop()
        }
    }

    /** r1d's view of the player (the song a station says is on comes from r1d's relay). */
    fun onState(player: JSONObject) {
        val song = player.optString("song")
        if (player.optString("app") == now.app && song != now.song) {
            set(now.copy(song = song), report = false)
        }
    }

    fun play(context: Context, appId: String, url: String, title: String, subtitle: String) {
        app = context.applicationContext
        release()
        set(Now(appId, "loading", title, subtitle, ""))
        val relayed = url.startsWith("http://127.0.0.1")
        io.execute {
            val token = if (relayed) R1d.token() else null
            main.post { open(url, token) }
        }
    }

    private fun open(url: String, token: String?) {
        val ctx = app ?: return
        if (now.state != "loading") return // stopped meanwhile
        val p = MediaPlayer()
        mp = p
        try {
            p.setAudioAttributes(ATTRS)
            p.setWakeMode(ctx, PowerManager.PARTIAL_WAKE_LOCK)
            val headers = if (token != null) mapOf("Authorization" to "Bearer $token") else emptyMap()
            p.setDataSource(ctx, Uri.parse(url), headers)
            p.setOnPreparedListener {
                if (mp !== it) return@setOnPreparedListener
                prepared = true
                if (now.state == "loading") start()
            }
            p.setOnErrorListener { mp, what, extra ->
                if (this.mp === mp) fail("couldn't play it ($what/$extra)")
                true
            }
            p.setOnInfoListener { mp, what, _ ->
                if (this.mp === mp) when (what) {
                    MediaPlayer.MEDIA_INFO_BUFFERING_START -> set(now.copy(state = "loading"))
                    MediaPlayer.MEDIA_INFO_BUFFERING_END -> if (mp.isPlaying) set(now.copy(state = "playing"))
                }
                false
            }
            p.setOnCompletionListener { if (mp === it) stop() }
            p.prepareAsync()
        } catch (e: Exception) {
            Log.w(TAG, "player: open", e)
            fail(e.message ?: "couldn't open it")
        }
    }

    private fun start() {
        val ctx = app ?: return
        if (!takeFocus(ctx)) {
            fail("another app is using the speaker")
            return
        }
        mp?.setVolume(level(), level())
        mp?.start()
        holdWifi(ctx, true)
        PlayerService.start(ctx)
        set(now.copy(state = "playing"))
    }

    fun pause() {
        val p = mp ?: return
        if (prepared && p.isPlaying) p.pause()
        pausedForFocus = false
        app?.let { holdWifi(it, false) }
        set(now.copy(state = "paused"))
    }

    fun resume() {
        val p = mp ?: return
        val ctx = app ?: return
        if (now.state != "paused") return
        if (!prepared) {
            set(now.copy(state = "loading"))
            return
        }
        if (!takeFocus(ctx)) return
        p.start()
        holdWifi(ctx, true)
        PlayerService.start(ctx)
        set(now.copy(state = "playing"))
    }

    fun toggle() = if (now.state == "playing" || now.state == "loading") pause() else resume()

    fun stop() {
        release()
        set(Now("", "stopped", "", "", ""))
    }

    /** You're holding the button to talk: the player dips, so the mic hears you and not it. */
    fun duck(on: Boolean) {
        if (dipped == on) return
        dipped = on
        mp?.setVolume(level(), level())
    }

    private fun level() = if (ducked || dipped) 0.15f else 1f

    private fun fail(why: String) {
        release()
        set(now.copy(state = "error"), error = why)
        // An error reads for a moment on the glance, then the player is free again.
        main.postDelayed({ if (now.state == "error") set(Now("", "stopped", "", "", "")) }, 6000)
    }

    private fun release() {
        mp?.let { runCatching { it.reset(); it.release() } }
        mp = null
        prepared = false
        pausedForFocus = false
        app?.let {
            dropFocus(it)
            holdWifi(it, false)
            PlayerService.stop(it)
        }
    }

    // ---- state ---------------------------------------------------------------------------------

    private fun set(n: Now, report: Boolean = true, error: String? = null) {
        now = n
        updateSession()
        listeners.forEach { it(n) }
        if (report) {
            val body = JSONObject().put("state", n.state)
            if (error != null) body.put("error", error)
            R1d.put("/api/player", body, onError = { Log.w(TAG, "player: report: $it") })
        }
    }

    private fun updateSession() {
        val ctx = app ?: return
        val s = session ?: MediaSession(ctx, "r1os").also {
            it.setCallback(object : MediaSession.Callback() {
                override fun onPlay() = resume()
                override fun onPause() = pause()
                override fun onStop() = stop()
            })
            session = it
        }
        val st = when (now.state) {
            "playing" -> PlaybackState.STATE_PLAYING
            "paused" -> PlaybackState.STATE_PAUSED
            "loading" -> PlaybackState.STATE_BUFFERING
            "error" -> PlaybackState.STATE_ERROR
            else -> PlaybackState.STATE_STOPPED
        }
        s.setPlaybackState(PlaybackState.Builder()
            .setActions(PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_STOP)
            .setState(st, PlaybackState.PLAYBACK_POSITION_UNKNOWN, 1f)
            .build())
        s.setMetadata(MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, now.song.ifEmpty { now.title })
            .putString(MediaMetadata.METADATA_KEY_ARTIST, if (now.song.isEmpty()) now.subtitle else now.title)
            .build())
        s.isActive = active
    }

    /** The session, for the service's notification. */
    fun sessionToken(): MediaSession.Token? = session?.sessionToken

    // ---- the system's share ----------------------------------------------------------------------

    private val ATTRS: AudioAttributes = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
        .build()

    private fun takeFocus(ctx: Context): Boolean {
        val am = ctx.getSystemService(AudioManager::class.java)
        val req = focus ?: AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(ATTRS)
            .setOnAudioFocusChangeListener({ change ->
                when (change) {
                    AudioManager.AUDIOFOCUS_LOSS -> pause()
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> if (now.state == "playing") {
                        pause()
                        pausedForFocus = true
                    }
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> {
                        ducked = true
                        mp?.setVolume(level(), level())
                    }
                    AudioManager.AUDIOFOCUS_GAIN -> {
                        ducked = false
                        mp?.setVolume(level(), level())
                        if (pausedForFocus) {
                            pausedForFocus = false
                            resume()
                        }
                    }
                }
            }, main)
            .build().also { focus = it }
        return am.requestAudioFocus(req) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
    }

    private fun dropFocus(ctx: Context) {
        focus?.let { ctx.getSystemService(AudioManager::class.java).abandonAudioFocusRequest(it) }
        ducked = false
    }

    /** Wi-Fi stays fully awake while a stream plays, or it drops packets with the screen off. */
    private fun holdWifi(ctx: Context, on: Boolean) {
        if (on) {
            if (wifi?.isHeld == true) return
            @Suppress("DEPRECATION")
            wifi = ctx.getSystemService(WifiManager::class.java)
                .createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "r1os:player").apply { setReferenceCounted(false); acquire() }
        } else {
            wifi?.let { if (it.isHeld) it.release() }
            wifi = null
        }
    }
}

/**
 * Keeps [Player] alive in the background (a foreground service of the media-playback type) while it plays or is
 * paused. Its notification has nowhere to show (the R1 has no status bar, ADR-0014); the glance screen is where you
 * see and control what's playing.
 */
class PlayerService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Playing", NotificationManager.IMPORTANCE_LOW))
        val n = Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(Player.now.title.ifEmpty { "Playing" })
            .setStyle(Notification.MediaStyle().setMediaSession(Player.sessionToken()))
            .setOngoing(true)
            .build()
        try {
            startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } catch (e: RuntimeException) {
            Log.w("r1os", "player service: not in the foreground", e)
            stopSelf()
        }
        return START_NOT_STICKY
    }

    companion object {
        private const val CHANNEL = "player"
        private const val ID = 2

        fun start(context: Context) {
            runCatching { context.startForegroundService(Intent(context, PlayerService::class.java)) }
                .onFailure { Log.w("r1os", "player service: start", it) }
        }

        fun stop(context: Context) {
            runCatching { context.stopService(Intent(context, PlayerService::class.java)) }
        }
    }
}
