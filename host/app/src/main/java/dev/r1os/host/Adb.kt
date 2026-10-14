package dev.r1os.host

import android.os.IBinder
import android.util.Log

/**
 * Answers Android's adb confirmations ([AdbConfirmActivity]) the way SystemUI's dialog does: through the `adb`
 * system service's IAdbManager. It's a hidden interface, reached by reflection: r1os's image exempts the host from
 * Android's hidden-API checks (hiddenapi-r1os.xml, from v0.2.10), and calling it needs MANAGE_DEBUGGING, a
 * privileged permission the image grants the host.
 */
object Adb {
    private const val TAG = "r1os"

    private fun service(): Any? = runCatching {
        val binder = Class.forName("android.os.ServiceManager").getMethod("getService", String::class.java).invoke(null, "adb") as? IBinder
            ?: return null
        Class.forName("android.debug.IAdbManager\$Stub").getMethod("asInterface", IBinder::class.java).invoke(null, binder)
    }.onFailure { Log.w(TAG, "adb: no IAdbManager", it) }.getOrNull()

    private fun call(name: String, types: Array<Class<*>>, vararg args: Any): Boolean {
        val s = service() ?: return false
        return runCatching {
            s.javaClass.getMethod(name, *types).invoke(s, *args)
            true
        }.onFailure { Log.w(TAG, "adb: $name", it) }.getOrDefault(false)
    }

    /** This computer's key may debug the R1: this time, or `always`. */
    fun allowKey(always: Boolean, key: String) = call("allowDebugging", arrayOf(Boolean::class.javaPrimitiveType!!, String::class.java), always, key)

    fun denyKey() = call("denyDebugging", arrayOf())

    /** Wireless debugging on this network (by its BSSID): this time, or `always`. */
    fun allowNetwork(always: Boolean, bssid: String) =
        call("allowWirelessDebugging", arrayOf(Boolean::class.javaPrimitiveType!!, String::class.java), always, bssid)

    fun denyNetwork() = call("denyWirelessDebugging", arrayOf())
}
