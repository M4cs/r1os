package dev.r1os.host.ui

import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.os.VibrationAttributes
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager

/**
 * The R1's haptics (Max, 2026-10-11: "it would feel nice if there was [feedback] to match the scroll and interaction"):
 * a touch from its vibration motor with each of the shell's sounds (Sounds), so the wheel ticks under your thumb and
 * choosing, bumping an end, listening and errors each have a feel. Sounds off doesn't turn them off; they're their
 * own setting (Settings › Display & sound, the companion, agents with your approval: `haptics`, `haptics_wheel`).
 *
 * The motor has no strength control (no amplitude capability, no primitives): how long it's driven is what you feel.
 * The system's prebaked effects don't climb with their names on the R1 (`dumpsys vibrator_manager`: TICK ~55–60 ms,
 * CLICK ~35–45, HEAVY_CLICK ~100), so medium's wheel ticks were longer than strong's (Max, 2026-10-11: "light haptics
 * is too light, medium is too much, and heavy is the same as medium"). Each level is pulses of its own lengths instead
 * ([FEELS]), even steps from light to strong. The wheel can be left out on its own (a spin is many ticks). As hardware
 * feedback, they play whatever the system's touch-feedback setting says, and cost next to nothing.
 */
object Haptics {
    val LEVELS = listOf("off", "light", "medium", "strong")
    private const val PREFS = "r1os"
    private const val KEY = "haptics"
    private const val WHEEL = "haptics_wheel"
    private var prefs: android.content.SharedPreferences? = null
    private var vibrator: Vibrator? = null
    private var lastTick = 0L

    /** A level's pulses, ms of motor: a wheel step, choosing (and the other actions), listening, and each of the two of
     *  an error or a reply. */
    private class Feel(val wheel: Long, val action: Long, val listen: Long, val double: Long)
    private val FEELS = mapOf(
        "light" to Feel(20, 28, 40, 22),
        "medium" to Feel(28, 36, 50, 30),
        "strong" to Feel(40, 60, 80, 42),
    )

    fun init(context: Context) {
        if (prefs != null) return
        prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        vibrator = if (Build.VERSION.SDK_INT >= 31) {
            context.getSystemService(VibratorManager::class.java)?.defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            context.getSystemService(Vibrator::class.java)
        }
    }

    /** off, light, medium (the default) or strong. */
    var level: String
        get() = prefs?.getString(KEY, "medium")?.takeIf { it in LEVELS } ?: "medium"
        set(v) { if (v in LEVELS) prefs?.edit()?.putString(KEY, v)?.apply() }

    /** Whether the wheel ticks too (on by default). */
    var wheel: Boolean
        get() = prefs?.getBoolean(WHEEL, true) ?: true
        set(v) { prefs?.edit()?.putBoolean(WHEEL, v)?.apply() }

    /** The feel of one of the shell's sounds, at the level set (nothing for those that don't have one). */
    fun feel(s: Sounds.S) {
        val f = FEELS[level] ?: return
        val v = vibrator ?: return
        if (!v.hasVibrator()) return
        when (s) {
            Sounds.S.TICK, Sounds.S.PULL1, Sounds.S.PULL2 -> {
                if (s == Sounds.S.TICK && !wheel) return
                // One at most every 50 ms, so a fast spin stays ticks, not a buzz.
                val now = SystemClock.uptimeMillis()
                if (now - lastTick < 50) return
                lastTick = now
                pulse(v, f.wheel)
            }
            Sounds.S.POP, Sounds.S.SEND, Sounds.S.OPEN, Sounds.S.BACK, Sounds.S.CLOSE, Sounds.S.BUMP -> pulse(v, f.action)
            Sounds.S.LISTEN -> pulse(v, f.listen)
            Sounds.S.ERROR, Sounds.S.NOTE -> play(v, VibrationEffect.createWaveform(longArrayOf(0, f.double, 70, f.double), -1))
            else -> return // waking and sleeping stay still
        }
    }

    /** The motor on for `ms` (it has no strength: on is all of it). */
    private fun pulse(v: Vibrator, ms: Long) = play(v, VibrationEffect.createOneShot(ms, VibrationEffect.DEFAULT_AMPLITUDE))

    private fun play(v: Vibrator, e: VibrationEffect) {
        if (Build.VERSION.SDK_INT >= 33) {
            v.vibrate(e, VibrationAttributes.createForUsage(VibrationAttributes.USAGE_HARDWARE_FEEDBACK))
        } else {
            v.vibrate(e)
        }
    }

    /** A taste of a level, as it's chosen: two wheel steps, then choosing. */
    fun preview() {
        val f = FEELS[level] ?: return
        val v = vibrator ?: return
        if (!v.hasVibrator()) return
        play(v, VibrationEffect.createWaveform(longArrayOf(0, f.wheel, 110, f.wheel, 160, f.action), -1))
    }

    /**
     * An alarm or a timer ringing (ADR-0050): the motor pulses with the chime (long, short, a rest: two seconds, like
     * it) until it stops, at any level but off. On the alarm's usage, so it's felt as an alarm is.
     */
    fun ring(on: Boolean) {
        val v = vibrator ?: return
        if (!on) {
            v.cancel()
            return
        }
        if (level == "off" || !v.hasVibrator()) return
        val e = VibrationEffect.createWaveform(longArrayOf(0, 320, 180, 120, 1380), 0)
        if (Build.VERSION.SDK_INT >= 33) {
            v.vibrate(e, VibrationAttributes.createForUsage(VibrationAttributes.USAGE_ALARM))
        } else {
            v.vibrate(e)
        }
    }
}
