package dev.r1os.host.ui

import android.content.Context
import android.media.AudioAttributes
import android.media.SoundPool
import android.os.SystemClock
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.PI
import kotlin.math.exp
import kotlin.math.ln
import kotlin.math.min
import kotlin.math.sin
import kotlin.random.Random

/**
 * The shell's UI sounds (ADR-0021): passive, not attention-grabbing (Max, 2026-10-09: "passive noises
 * rather than super overstimulating"). Soft, low, short pops for actions and quiet ticks for the
 * wheel; the unlock is two barely-there taps and one soft landing, and waking makes no sound. Each is
 * synthesized here (a sine gliding in pitch, a soft attack, an exponential tail) and written to the
 * cache once, so there are no audio files to license or ship. They play through SoundPool on the
 * media stream, the one hold + wheel sets (ADR-0022): one volume for everything the R1 plays (Max,
 * 2026-10-09: they used Android's UI-sound channel, so the volume didn't reach them).
 */
object Sounds {
    enum class S(val gain: Float) {
        /** The wheel moved the focus. */
        TICK(0.16f),
        /** Something was chosen. */
        POP(0.3f),
        /** Back to where you were. */
        BACK(0.26f),
        /** A sheet or the shell opened up (the launcher, home from the glance screen). */
        OPEN(0.24f),
        CLOSE(0.2f),
        /** A resisting pull, a step at a time. */
        PULL1(0.14f),
        PULL2(0.16f),
        /** The wheel hit an end (Back peeks over a header). */
        BUMP(0.16f),
        /** Push-to-talk is listening. */
        LISTEN(0.2f),
        /** The words went. */
        SEND(0.26f),
        ERROR(0.26f),
        /** Off to sleep. */
        SLEEP(0.2f),
        /** Unused for now: waking stays silent. */
        WAKE(0.12f),
        /** "Nova replied." */
        NOTE(0.22f),
    }

    private var pool: SoundPool? = null
    /** The volume note's own pool, so a spin of ticks never cuts it off; the same stream. */
    private var mediaPool: SoundPool? = null
    private var levelId = 0
    private var levelReady = false
    private val ids = HashMap<S, Int>()
    private val ready = HashSet<Int>()
    private var lastTick = 0L
    private const val PREFS = "r1os"
    private const val KEY = "sounds"
    private var prefs: android.content.SharedPreferences? = null

    /** Sounds on or off (Settings; on by default). */
    var enabled: Boolean
        get() = prefs?.getBoolean(KEY, true) ?: true
        set(v) { prefs?.edit()?.putBoolean(KEY, v)?.apply() }

    fun init(context: Context) {
        if (pool != null) return
        prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val p = SoundPool.Builder()
            .setMaxStreams(6)
            .setAudioAttributes(AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build())
            .build()
        p.setOnLoadCompleteListener { _, id, status -> if (status == 0) ready += id }
        pool = p
        val dir = File(context.cacheDir, "sounds-$VERSION").apply { mkdirs() }
        val m = SoundPool.Builder()
            .setMaxStreams(2)
            .setAudioAttributes(AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build())
            .build()
        m.setOnLoadCompleteListener { _, _, status -> if (status == 0) levelReady = true }
        mediaPool = m
        Thread({
            val lf = File(dir, "level.wav")
            if (!lf.exists()) writeWav(lf, blip(520f, 600f, 90, 30f, harmonic = 0.06f))
            levelId = m.load(lf.path, 1)
            for (s in S.entries) {
                val f = File(dir, "${s.name.lowercase()}.wav")
                if (!f.exists()) writeWav(f, synth(s))
                val id = p.load(f.path, 1)
                synchronized(ids) { ids[s] = id }
            }
        }, "r1os-sounds").start()
    }

    // ---- the alarm (ADR-0028) ------------------------------------------------------------------------
    // Its own pool on the alarm stream, looping until stopped, and it rings even with the UI sounds off.
    // It starts soft and grows over half a minute. The R1 has one volume (the media stream's, ADR-0022), so
    // the alarm stream follows it, but never below half: an alarm turned down to nothing would be no alarm.

    private var alarmPool: SoundPool? = null
    private var alarmId = 0
    private var chimeReady = false
    /** What waits for the chime to load. */
    private var onChime: (() -> Unit)? = null
    private var alarmStream = 0
    private var alarmStarted = 0L
    private val main = android.os.Handler(android.os.Looper.getMainLooper())
    private val swell = object : Runnable {
        override fun run() {
            if (alarmStream == 0) return
            val g = (0.25f + 0.75f * (SystemClock.uptimeMillis() - alarmStarted) / SWELL_MS).coerceAtMost(1f)
            alarmPool?.setVolume(alarmStream, g, g)
            if (g < 1f) main.postDelayed(this, 1000)
        }
    }

    /** Starts the alarm's tone (looping), or stops it. */
    fun alarm(context: Context, on: Boolean) {
        main.removeCallbacks(swell)
        alarmPool?.let { p -> if (alarmStream != 0) p.stop(alarmStream) }
        alarmStream = 0
        if (!on) {
            alarmStarted = 0L
            onChime = null
            return
        }
        floor(context)
        alarmStarted = SystemClock.uptimeMillis()
        withChime(context) { p ->
            if (alarmStarted == 0L) return@withChime
            alarmStream = p.play(alarmId, 0.25f, 0.25f, 2, -1, 1f)
            main.postDelayed(swell, 1000)
        }
    }

    /**
     * The alarm's chime once (a pomodoro's phase ended, ADR-0050), on the alarm stream at the alarm's floor: with UI
     * sounds off too, as a timer is. Not over a ringing alarm.
     */
    fun chime(context: Context) {
        if (alarmStream != 0) return
        floor(context)
        withChime(context) { p -> if (alarmStream == 0) p.play(alarmId, 0.7f, 0.7f, 1, 0, 1f) }
    }

    /** The alarm stream follows the R1's one volume, but never below half. */
    private fun floor(context: Context) {
        val audio = context.getSystemService(android.media.AudioManager::class.java)
        val media = audio.getStreamVolume(android.media.AudioManager.STREAM_MUSIC).toFloat() /
            audio.getStreamMaxVolume(android.media.AudioManager.STREAM_MUSIC).coerceAtLeast(1)
        val max = audio.getStreamMaxVolume(android.media.AudioManager.STREAM_ALARM)
        runCatching {
            audio.setStreamVolume(android.media.AudioManager.STREAM_ALARM, (max * media.coerceAtLeast(0.5f)).toInt().coerceIn(1, max), 0)
        }
    }

    /** The alarm's pool with its chime loaded (once), then `then`. */
    private fun withChime(context: Context, then: (SoundPool) -> Unit) {
        val p = alarmPool ?: SoundPool.Builder()
            .setMaxStreams(2)
            .setAudioAttributes(AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_ALARM)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build())
            .build().also { alarmPool = it }
        if (chimeReady) {
            then(p)
            return
        }
        onChime = { then(p) }
        if (alarmId != 0) return // loading
        val f = File(context.cacheDir, "sounds-$VERSION/alarm.wav")
        if (!f.exists()) {
            f.parentFile?.mkdirs()
            writeWav(f, chime())
        }
        p.setOnLoadCompleteListener { _, id, status ->
            if (status == 0 && id == alarmId) {
                chimeReady = true
                onChime?.invoke()
                onChime = null
            }
        }
        alarmId = p.load(f.path, 1)
    }

    /** Three bright notes, twice, then a breath: about two seconds, looped. */
    private fun chime(): FloatArray {
        val note = { f: Float -> blip(f, f, 260, 90f, harmonic = 0.12f) }
        return mix(
            0 to note(784f), 150 to note(988f), 300 to note(1175f),
            650 to note(784f), 800 to note(988f), 950 to note(1175f),
            1900 to FloatArray(1),
        )
    }

    /** The volume's note: higher the louder, played on the media stream at the new level. */
    fun level(value: Int, max: Int) {
        if (!enabled || !levelReady) return
        val f = if (max > 0) value.toFloat() / max else 0f
        mediaPool?.play(levelId, 0.7f, 0.7f, 1, 0, 0.7f + 0.8f * f)
    }

    /** Plays `s` (a tick at most every 18 ms: a fast spin shouldn't turn into a buzz). */
    fun play(s: S) {
        // Its feel, whether sounds are on or not (Haptics: its own setting).
        Haptics.feel(s)
        if (!enabled) return
        val p = pool ?: return
        val id = synchronized(ids) { ids[s] } ?: return
        if (id !in ready) return
        var rate = 1f
        if (s == S.TICK) {
            val now = SystemClock.uptimeMillis()
            if (now - lastTick < 18) return
            lastTick = now
            rate = 0.97f + Random.nextFloat() * 0.06f // a little life, so a spin doesn't sound mechanical
        }
        p.play(id, s.gain, s.gain, 1, 0, rate)
    }

    // ---- synthesis ------------------------------------------------------------------------------

    private const val VERSION = 2
    /** How long the alarm takes to grow to full volume. */
    private const val SWELL_MS = 30_000f
    private const val RATE = 44100

    /** A sine gliding from `f0` to `f1` Hz over `ms`, with a soft attack and an exponential tail. */
    private fun blip(f0: Float, f1: Float, ms: Int, tauMs: Float, harmonic: Float = 0.18f, click: Float = 0f): FloatArray {
        val n = RATE * ms / 1000
        val out = FloatArray(n)
        var phase = 0.0
        val attack = RATE * 0.003
        for (i in 0 until n) {
            val t = i.toDouble() / n
            // Exponential glide sounds natural (pitch is logarithmic).
            val f = f0 * exp(ln((f1 / f0).toDouble()) * t)
            phase += 2 * PI * f / RATE
            val env = min(1.0, i / attack) * exp(-i / (RATE * tauMs / 1000.0))
            var v = sin(phase) + harmonic * sin(2 * phase)
            if (click > 0 && i < RATE / 1000) v += click * (Random.nextFloat() * 2 - 1) * (1 - i / (RATE / 1000.0))
            out[i] = (v * env).toFloat()
        }
        return out
    }

    private fun mix(vararg parts: Pair<Int, FloatArray>): FloatArray {
        val len = parts.maxOf { (at, a) -> RATE * at / 1000 + a.size }
        val out = FloatArray(len)
        for ((at, a) in parts) {
            val o = RATE * at / 1000
            for (i in a.indices) out[o + i] += a[i]
        }
        return out
    }

    // Low and short, with small glides: felt more than heard.
    private fun synth(s: S): FloatArray = when (s) {
        S.TICK -> blip(1800f, 1650f, 16, 3.2f, harmonic = 0.03f, click = 0.12f)
        S.POP -> blip(380f, 560f, 55, 14f, harmonic = 0.08f)
        S.BACK -> blip(560f, 400f, 60, 15f, harmonic = 0.08f)
        S.OPEN -> blip(330f, 460f, 80, 22f, harmonic = 0.06f)
        S.CLOSE -> blip(460f, 330f, 80, 22f, harmonic = 0.06f)
        S.PULL1 -> blip(250f, 280f, 40, 9f, harmonic = 0.04f)
        S.PULL2 -> blip(280f, 320f, 40, 9f, harmonic = 0.04f)
        S.BUMP -> blip(230f, 250f, 36, 8f, harmonic = 0.04f)
        S.LISTEN -> blip(300f, 470f, 90, 30f, harmonic = 0.05f)
        S.SEND -> blip(400f, 620f, 65, 18f, harmonic = 0.06f)
        S.ERROR -> blip(220f, 175f, 120, 40f, harmonic = 0.25f)
        S.SLEEP -> blip(420f, 260f, 150, 50f, harmonic = 0.05f)
        S.WAKE -> blip(260f, 380f, 120, 40f, harmonic = 0.05f)
        S.NOTE -> mix(0 to blip(660f, 660f, 200, 60f, harmonic = 0.04f), 70 to blip(880f, 880f, 220, 65f, harmonic = 0.03f))
    }

    /** 16-bit mono PCM WAV, normalised to the peak so every sound's `gain` means the same thing. */
    private fun writeWav(f: File, samples: FloatArray) {
        val peak = samples.maxOf { kotlin.math.abs(it) }.coerceAtLeast(1e-6f)
        val data = ByteBuffer.allocate(samples.size * 2).order(ByteOrder.LITTLE_ENDIAN)
        for (v in samples) data.putShort(((v / peak) * 0.9f * Short.MAX_VALUE).toInt().toShort())
        val h = ByteBuffer.allocate(44).order(ByteOrder.LITTLE_ENDIAN)
        h.put("RIFF".toByteArray()).putInt(36 + data.capacity()).put("WAVE".toByteArray())
        h.put("fmt ".toByteArray()).putInt(16).putShort(1).putShort(1).putInt(RATE).putInt(RATE * 2).putShort(2).putShort(16)
        h.put("data".toByteArray()).putInt(data.capacity())
        val tmp = File(f.path + ".tmp")
        FileOutputStream(tmp).use { it.write(h.array()); it.write(data.array()) }
        tmp.renameTo(f)
    }
}
