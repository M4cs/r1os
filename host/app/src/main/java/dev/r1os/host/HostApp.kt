package dev.r1os.host

import android.app.Application
import android.content.Context
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.shell.Shell
import org.json.JSONObject

/**
 * The host's process (T-142): r1d's events from the moment it starts, whatever started it: the home screen, push-to-talk's
 * service, the keyboard, the notification listener or the app window's reader.
 */
class HostApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Requests.start(this)
    }
}

/**
 * No status bar, notification shade (EXPAND covers quick settings) or heads-up alerts (ADR-0014, ADR-0020): the R1 has
 * nothing to pull down. SystemUI is still in the image (Q22's first cut kept it), so the host turns them off with
 * StatusBarManager's `disable`, which needs STATUS_BAR (privileged; r1os's image grants it) and isn't public API: best
 * effort. It holds while this process lives, on the display of the context it's asked from (the R1's own: the
 * application's), so the process asks as it starts, and the home screen again as it comes back.
 */
object AndroidBars {
    // StatusBarManager: EXPAND | NOTIFICATION_ICONS | NOTIFICATION_ALERTS | SYSTEM_INFO | CLOCK | ONGOING_CALL_CHIP.
    private const val FLAGS = 0x00010000 or 0x00020000 or 0x00040000 or 0x00100000 or 0x00800000 or 0x04000000

    fun off(context: Context) {
        val sbm = context.applicationContext.getSystemService("statusbar") ?: return
        runCatching {
            sbm.javaClass.getMethod("disable", Int::class.javaPrimitiveType).invoke(sbm, FLAGS)
        }.onFailure { android.util.Log.w("r1os", "status bar: disable", it) }
    }
}

/**
 * Agents' requests that need no screen of r1os's (T-142): the app window (ADR-0061), the browser (ADR-0054) and the CPU held
 * for a routine. The shell took them until T-142, so a process started without it (after a crash or an update, with an
 * Android app in front) never answered, and an agent's `app_open` failed after 30 s with "the R1's app window didn't
 * answer".
 *
 * - **The app window** needs nothing of the shell's: it answers at once. The shell, when there is one, shows it as the
 *   agent works ([Shell.request]); an ask with no shell brings r1os forward, and the shell shows it as it comes.
 * - **The browser's page** lives in the shell (behind everything, so it lays out and draws): with no shell, r1os comes
 *   forward and the request waits for it.
 */
object Requests {
    private lateinit var app: Context
    private var shell: Shell? = null
    private val waiting = ArrayList<Ev>()
    /** The app window asked for you while there was no shell to show it. */
    private var missedAsk: Ev? = null

    fun start(context: Context) {
        app = context.applicationContext
        // From the process's start, not only the home screen's: a host that crashed with an Android app in front comes
        // back in the background, and Android's bars were back until you went home (2026-10-13).
        AndroidBars.off(app)
        R1d.init(app)
        // A reader left on by an earlier run (a crash) goes off.
        AgentApps.init(app)
        R1d.onEvent(::handle)
        R1d.start()
    }

    /** The shell is up: it shows what it missed, and the browser's requests that waited for it go ahead. */
    fun attach(s: Shell) {
        shell = s
        missedAsk?.takeIf { AgentApps.asking != null }?.let { s.request(it) }
        missedAsk = null
        val w = ArrayList(waiting)
        waiting.clear()
        w.forEach(::handle)
    }

    fun detach(s: Shell) {
        if (shell === s) shell = null
    }

    private fun handle(ev: Ev) {
        val s = shell
        when (ev.type) {
            "app_window" -> {
                AgentApps.handle(app, ev)
                when {
                    s != null -> s.request(ev)
                    ev.str("op") == "ask_user" -> { missedAsk = ev; comeForward() }
                }
            }
            "browser" -> when {
                // Made in the shell's own context (its density), and parked behind it.
                s != null -> { AgentBrowser.handle(s.context, ev); s.request(ev) }
                ev.str("op") == "stopped" -> AgentBrowser.handle(app, ev)
                else -> {
                    if (waiting.isEmpty()) comeForward()
                    waiting += ev
                }
            }
            // The CPU held awake while a routine runs with the screen off (R7).
            "awake" -> Awake.hold(app, ev.str("why").orEmpty(), ev.bool("on"))
        }
    }

    /** r1os's home comes to the front, as the shell's `comeBack` does it (r1d starts Android's home). */
    private fun comeForward() = R1d.post("/api/android/open", JSONObject().put("screen", "home"))
}
