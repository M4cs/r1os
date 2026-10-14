package dev.r1os.host

import android.content.Context
import android.database.ContentObserver
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import org.json.JSONObject
import kotlin.math.roundToInt

/**
 * The R1's own settings that are Android's (ADR-0045): the display's brightness (and whether it's automatic), how
 * long until the screen sleeps, the media volume, and r1os's sounds. Settings › Display & sound changes them here;
 * agents change them through r1d (`device_set`, after you approve). Each change, and each made elsewhere (Android's
 * settings, hold + wheel), is reported to r1d (`POST /api/device`), so agents read what's true.
 *
 * Brightness, the timeout and the 24-hour clock need WRITE_SETTINGS: a preinstalled app's, and r1d grants its app-op if
 * Android hasn't. Docked mode is r1os's own (the glance screen keeps the display on while it charges).
 */
object DeviceSettings {
    private const val DOCKED = "docked"

    /** Docked mode: while it charges, the glance screen keeps the display on (a bedside clock). Off by default. */
    val docked: Boolean get() = app?.getSharedPreferences("r1os", Context.MODE_PRIVATE)?.getBoolean(DOCKED, false) ?: false

    /** The shell hears when docked mode changes. */
    var onDocked: () -> Unit = {}

    /** How long the display stays on (seconds): r1d's TIMEOUTS. */
    val TIMEOUTS = listOf(15, 30, 60, 120, 300, 600)

    private val main = Handler(Looper.getMainLooper())
    private var app: Context? = null
    private var observer: ContentObserver? = null
    private val report = Runnable { app?.let { R1d.post("/api/device", read(it)) } }

    /** Watches Android's own settings for changes made elsewhere (no polling: Android calls back). */
    fun init(context: Context) {
        if (observer != null) return
        val ctx = context.applicationContext
        app = ctx
        val o = object : ContentObserver(main) {
            override fun onChange(selfChange: Boolean) = reportSoon()
        }
        for (k in listOf(Settings.System.SCREEN_BRIGHTNESS, Settings.System.SCREEN_BRIGHTNESS_MODE, Settings.System.SCREEN_OFF_TIMEOUT, Settings.System.TIME_12_24)) {
            ctx.contentResolver.registerContentObserver(Settings.System.getUriFor(k), false, o)
        }
        observer = o
    }

    /** r1d hears the settings now (on connecting). Asks r1d for the settings app-op if Android hasn't given it. */
    fun connected(context: Context) {
        if (!Settings.System.canWrite(context)) R1d.post("/api/android/settings-access")
        main.removeCallbacks(report)
        R1d.post("/api/device", read(context))
    }

    /** r1d hears them in a moment: a burst of changes (the wheel turning brightness) is told once. */
    fun reportSoon() {
        main.removeCallbacks(report)
        main.postDelayed(report, REPORT_MS)
    }

    fun read(context: Context): JSONObject {
        val cr = context.contentResolver
        val audio = context.getSystemService(AudioManager::class.java)
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
        return JSONObject()
            .put("brightness", (Settings.System.getInt(cr, Settings.System.SCREEN_BRIGHTNESS, 128) * 100f / 255f).roundToInt())
            .put("auto_brightness", Settings.System.getInt(cr, Settings.System.SCREEN_BRIGHTNESS_MODE, 0) == Settings.System.SCREEN_BRIGHTNESS_MODE_AUTOMATIC)
            .put("screen_timeout", Settings.System.getInt(cr, Settings.System.SCREEN_OFF_TIMEOUT, 60_000) / 1000)
            .put("volume", (audio.getStreamVolume(AudioManager.STREAM_MUSIC) * 100f / max).roundToInt())
            .put("sounds", Sounds.enabled)
            .put("haptics", dev.r1os.host.ui.Haptics.level)
            .put("haptics_wheel", dev.r1os.host.ui.Haptics.wheel)
            .put("clock_24h", android.text.format.DateFormat.is24HourFormat(context))
            .put("bluetooth", Bluetooth.on)
            .put(DOCKED, context.getSharedPreferences("r1os", Context.MODE_PRIVATE).getBoolean(DOCKED, false))
            .put("dns", dns(context))
    }

    /** The active network's DNS servers: r1d's link looks names up there (it can't ask Android itself, ADR-0048). */
    private fun dns(context: Context): org.json.JSONArray {
        val cm = context.getSystemService(android.net.ConnectivityManager::class.java)
        val servers = cm.getLinkProperties(cm.activeNetwork)?.dnsServers.orEmpty()
        return org.json.JSONArray(servers.mapNotNull { it.hostAddress })
    }

    /** Makes the changes r1d sends (`device_set {changes}`) or Settings makes; then tells r1d what they are now. */
    fun apply(context: Context, changes: JSONObject) {
        val cr = context.contentResolver
        val canWrite = Settings.System.canWrite(context)
        fun write(key: String, v: Int) {
            if (canWrite) Settings.System.putInt(cr, key, v)
            else android.util.Log.w("r1os", "can't write $key: no WRITE_SETTINGS yet")
        }
        if (changes.has("auto_brightness")) {
            write(Settings.System.SCREEN_BRIGHTNESS_MODE,
                if (changes.optBoolean("auto_brightness")) Settings.System.SCREEN_BRIGHTNESS_MODE_AUTOMATIC else Settings.System.SCREEN_BRIGHTNESS_MODE_MANUAL)
        }
        if (changes.has("brightness")) {
            // 0% is the dimmest the display goes, never off.
            write(Settings.System.SCREEN_BRIGHTNESS, (changes.optInt("brightness").coerceIn(0, 100) * 255f / 100f).roundToInt().coerceIn(1, 255))
        }
        if (changes.has("screen_timeout")) {
            val s = changes.optInt("screen_timeout")
            if (s in TIMEOUTS) write(Settings.System.SCREEN_OFF_TIMEOUT, s * 1000)
        }
        if (changes.has("volume")) {
            val audio = context.getSystemService(AudioManager::class.java)
            val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
            audio.setStreamVolume(AudioManager.STREAM_MUSIC, (changes.optInt("volume").coerceIn(0, 100) * max / 100f).roundToInt(), 0)
        }
        if (changes.has("sounds")) Sounds.enabled = changes.optBoolean("sounds")
        // The clocks (TextClock) follow Android's own setting by themselves.
        if (changes.has("clock_24h") && canWrite) Settings.System.putString(cr, Settings.System.TIME_12_24, if (changes.optBoolean("clock_24h")) "24" else "12")
        if (changes.has("bluetooth") && changes.optBoolean("bluetooth") != Bluetooth.on) Bluetooth.setOn(context, changes.optBoolean("bluetooth"))
        if (changes.has(DOCKED)) {
            context.getSharedPreferences("r1os", Context.MODE_PRIVATE).edit().putBoolean(DOCKED, changes.optBoolean(DOCKED)).apply()
            main.post { onDocked() }
        }
        if (changes.has("haptics_wheel")) dev.r1os.host.ui.Haptics.wheel = changes.optBoolean("haptics_wheel")
        if (changes.has("haptics")) {
            dev.r1os.host.ui.Haptics.level = changes.optString("haptics")
            dev.r1os.host.ui.Haptics.preview()
        }
        reportSoon()
    }

    private const val REPORT_MS = 600L
}
