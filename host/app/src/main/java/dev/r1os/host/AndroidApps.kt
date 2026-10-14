package dev.r1os.host

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.graphics.drawable.Drawable
import android.os.Build
import android.util.Log
import dev.r1os.host.net.R1d
import org.json.JSONArray
import org.json.JSONObject

/**
 * The Android apps installed on the R1 (ADR-0013, roadmap R4): what the launcher lists, opening one, and Aurora
 * Store, the R1's app store. Only an app can read their names, so the host also sends r1d the list (for agents'
 * tools and System One, `PUT /api/android/apps`) when it connects and when one is installed or removed.
 */
object AndroidApps {
    private const val TAG = "r1os"
    const val STORE = "com.aurora.store"

    class App(val pkg: String, val name: String, val system: Boolean, private val info: ApplicationInfo) {
        fun icon(context: Context): Drawable? = runCatching { context.packageManager.getApplicationIcon(info) }.getOrNull()
    }

    /** Everything with a launcher entry but the host, by name. */
    fun list(context: Context): List<App> {
        val pm = context.packageManager
        val main = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        return pm.queryIntentActivities(main, 0)
            .map { it.activityInfo.applicationInfo }
            .filter { it.packageName != context.packageName }
            .distinctBy { it.packageName }
            .map { App(it.packageName, pm.getApplicationLabel(it).toString(), it.flags and ApplicationInfo.FLAG_SYSTEM != 0, it) }
            .sortedBy { it.name.lowercase() }
    }

    fun has(context: Context, pkg: String) =
        runCatching { context.packageManager.getApplicationInfo(pkg, 0) }.isSuccess

    /** Tells r1d what's installed. */
    fun publish(context: Context) {
        val apps = JSONArray()
        for (a in list(context)) apps.put(JSONObject().put("package", a.pkg).put("name", a.name).put("system", a.system))
        R1d.put("/api/android/apps", JSONObject().put("apps", apps), onError = { Log.w(TAG, "android apps: $it") })
    }

    /** Opens an app as a launcher does: its task comes back if it has one. */
    fun open(context: Context, pkg: String): Boolean {
        val i = context.packageManager.getLaunchIntentForPackage(pkg) ?: return false
        i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED)
        return runCatching { context.startActivity(i) }.onFailure { Log.w(TAG, "open $pkg", it) }.isSuccess
    }

    private var watching: (() -> Unit)? = null

    /** Calls [onChange] (on the main thread) when an app is installed, updated or removed. Registered once per process: a
     *  shell built again (a change of look) takes the callback over. */
    fun watch(context: Context, onChange: () -> Unit) {
        val first = watching == null
        watching = onChange
        if (!first) return
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_PACKAGE_ADDED)
            addAction(Intent.ACTION_PACKAGE_REMOVED)
            addAction(Intent.ACTION_PACKAGE_CHANGED)
            addAction(Intent.ACTION_PACKAGE_REPLACED)
            addDataScheme("package")
        }
        val r = object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) {
                // An update sends removed then added: the added is enough.
                if (i.action == Intent.ACTION_PACKAGE_REMOVED && i.getBooleanExtra(Intent.EXTRA_REPLACING, false)) return
                watching?.invoke()
            }
        }
        val app = context.applicationContext
        if (Build.VERSION.SDK_INT >= 33) app.registerReceiver(r, filter, Context.RECEIVER_NOT_EXPORTED) else app.registerReceiver(r, filter)
    }
}
