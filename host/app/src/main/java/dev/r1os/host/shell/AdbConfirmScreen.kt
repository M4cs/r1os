package dev.r1os.host.shell

import dev.r1os.host.Adb
import dev.r1os.host.ui.Theme

/**
 * Android asks whether a computer may debug the R1 over adb (its key), or whether wireless debugging may stay on
 * for a network: r1os answers here, in its own look, so SystemUI's dialog isn't needed (roadmap R4's keep-list).
 * [dev.r1os.host.AdbConfirmActivity] brings the request. Leaving without an answer is a no, as SystemUI's is.
 */
class AdbConfirmScreen(
    shell: Shell,
    private val key: String?,
    private val fingerprints: String?,
    ssid: String?,
    private val bssid: String?,
) : ListPage(shell, if (key != null) "Allow this computer?" else "Debug over Wi-Fi?") {
    /** The network's name: Android's comes in quotes, or as `<unknown ssid>` without location access. */
    private val network = ssid?.removeSurrounding("\"")?.takeIf { it.isNotBlank() && it != "<unknown ssid>" }
    private var answered = false

    init {
        render()
    }

    private fun answer(done: () -> Boolean) {
        answered = true
        done()
        shell.back()
    }

    private fun render(): Unit = list.rebuild {
        if (key != null) {
            note("A computer wants to debug the R1 over adb. Its key's fingerprint:")
            note(fingerprints.orEmpty(), color = Theme.FG, size = 13f)
            item("check-circle", "always allow it", accent) { answer { Adb.allowKey(true, key) } }
            item("check", "allow this time") { answer { Adb.allowKey(false, key) } }
            item("x", "don't allow", danger = true) { answer { Adb.denyKey() } }
        } else {
            note("Wireless debugging wants to stay on for ${network ?: "this network"}.")
            val b = bssid.orEmpty()
            item("check-circle", "always on this network", accent) { answer { Adb.allowNetwork(true, b) } }
            item("check", "this time") { answer { Adb.allowNetwork(false, b) } }
            item("x", "don't allow", danger = true) { answer { Adb.denyNetwork() } }
        }
    }

    override fun onDestroy() {
        if (answered) return
        if (key != null) Adb.denyKey() else Adb.denyNetwork()
    }
}
