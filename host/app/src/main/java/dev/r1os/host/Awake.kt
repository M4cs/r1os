package dev.r1os.host

import android.content.Context
import android.os.PowerManager

/**
 * The CPU held awake while r1d runs something with the screen off (`awake {why, on}`): a routine's turn (R7). r1d's
 * kernel alarm wakes the R1 for it and holds it 30 s; this holds it for the turn itself, five minutes at most, so a
 * stuck turn can't keep the R1 up.
 */
object Awake {
    private val held = HashMap<String, PowerManager.WakeLock>()

    fun hold(context: Context, why: String, on: Boolean) {
        held.remove(why)?.let { if (it.isHeld) it.release() }
        if (!on) return
        val pm = context.getSystemService(PowerManager::class.java) ?: return
        val lock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "r1os:$why").apply { setReferenceCounted(false) }
        lock.acquire(MAX_MS)
        held[why] = lock
    }

    private const val MAX_MS = 5 * 60_000L
}
