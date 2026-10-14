package dev.r1os.host

import android.app.ActivityManager
import android.app.usage.UsageStatsManager
import android.content.Context
import android.hardware.display.DisplayManager
import android.media.session.MediaSessionManager
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.Display

/**
 * Android apps you've left alone close in the background (T-150). Android keeps an app you've left cached until it needs
 * the memory, and on the R1 (3.9 GB, with Chrome's web pages and agents' apps) the time it waited meant swapping and
 * stutters under your hands. So a cached app you haven't used for [IDLE_MS] is closed with Android's own
 * `killBackgroundProcesses`: the same as its memory killer, sooner. It never touches what's on the screen, a foreground
 * service, or a persistent process, and r1os also keeps what's playing (or paused) in a media session, the keyboard,
 * and the app in an agent's window.
 *
 * Looked at [AFTER_OFF_MS] after the R1's screen goes off, and [IDLE_MS] after you come back to r1os from an app: never
 * on a timer of its own, and nothing with the screen off after that one look.
 */
object BackgroundApps {
    private const val TAG = "r1os"
    private const val IDLE_MS = 10 * 60_000L
    private const val AFTER_OFF_MS = 30_000L

    private val main = Handler(Looper.getMainLooper())
    private lateinit var app: Context
    private var off = false
    private var started = false

    fun init(context: Context) {
        if (started) return
        started = true
        app = context.applicationContext
        val dm = app.getSystemService(DisplayManager::class.java)
        dm.registerDisplayListener(object : DisplayManager.DisplayListener {
            override fun onDisplayChanged(id: Int) {
                if (id != Display.DEFAULT_DISPLAY) return
                val nowOff = dm.getDisplay(id)?.state != Display.STATE_ON
                if (nowOff == off) return
                off = nowOff
                main.removeCallbacks(afterOff)
                if (off) main.postDelayed(afterOff, AFTER_OFF_MS)
            }
            override fun onDisplayAdded(id: Int) {}
            override fun onDisplayRemoved(id: Int) {}
        }, main)
    }

    /** r1os came to the front again (you left an app, or woke the R1): a look once that app's been idle long enough. */
    fun onHome() {
        if (!started) return
        main.removeCallbacks(afterHome)
        main.postDelayed(afterHome, IDLE_MS)
    }

    private val afterOff = Runnable { look(screenOff = true) }
    private val afterHome = Runnable { look(screenOff = false) }

    /** Closes what's cached and idle, off the UI thread (the calls go to Android's services). */
    private fun look(screenOff: Boolean) {
        Thread({ runCatching { sweep(screenOff) }.onFailure { Log.w(TAG, "background apps: the look failed", it) } }, "r1os-background-apps").start()
    }

    private fun sweep(screenOff: Boolean) {
        val am = app.getSystemService(ActivityManager::class.java)
        val now = System.currentTimeMillis()
        // When each app was last used. Without that (no usage access), only with the screen off, when nothing's in use.
        val used = runCatching {
            app.getSystemService(UsageStatsManager::class.java).queryAndAggregateUsageStats(now - 24 * 3_600_000L, now)
        }.getOrNull().orEmpty()
        if (used.isEmpty() && !screenOff) return
        val playing = runCatching {
            app.getSystemService(MediaSessionManager::class.java).getActiveSessions(null).map { it.packageName }
        }.getOrDefault(emptyList())
        val keyboard = Settings.Secure.getString(app.contentResolver, Settings.Secure.DEFAULT_INPUT_METHOD)?.substringBefore('/')
        val keep = setOf(app.packageName) + playing + listOfNotNull(keyboard, AgentApps.pkg)
        val cached = am.runningAppProcesses.orEmpty().filter { it.importance >= ActivityManager.RunningAppProcessInfo.IMPORTANCE_CACHED }
        val before = memoryAvailableMb(am)
        val tried = cached.flatMap { it.pkgList.orEmpty().toList() }.toSet()
            .filter { it !in keep && now - (used[it]?.lastTimeUsed ?: 0L) >= IDLE_MS }
            .onEach { runCatching { am.killBackgroundProcesses(it) } }
        if (tried.isEmpty()) return
        // What's actually gone, and the memory back, a moment later (the kills land as Android gets to them). From Android
        // 14 they need KILL_ALL_BACKGROUND_PROCESSES: without it, nothing closed and this said otherwise (2026-10-13).
        Thread.sleep(2_000)
        val still = am.runningAppProcesses.orEmpty().flatMap { it.pkgList.orEmpty().toList() }.toSet()
        val closed = tried.filter { it !in still }
        Log.i(TAG, "background apps: closed ${closed.size} of ${tried.size} (${closed.joinToString()}); available $before MB before, ${memoryAvailableMb(am)} MB after")
    }

    private fun memoryAvailableMb(am: ActivityManager) = ActivityManager.MemoryInfo().also { am.getMemoryInfo(it) }.availMem / 1_048_576
}
