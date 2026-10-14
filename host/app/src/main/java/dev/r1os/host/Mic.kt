package dev.r1os.host

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.util.Log
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket

/**
 * The R1's microphone, lent to r1d for push-to-talk (ADR-0009; crate::voice in r1d).
 *
 * Android only lets a foreground app record, so the host owns the mic and r1d asks for it. This
 * thread keeps a loopback connection to r1d (127.0.0.1:7173), sends `r1os-mic 1 16000`, and records
 * 16 kHz mono s16le PCM while r1d has sent `S`, until `E`. r1d does the rest: speech-to-text and the
 * events the shell draws. Nothing is stored here; the audio only crosses the loopback socket.
 */
class Mic : Thread("r1os-mic") {
    @Volatile private var running = true
    @Volatile private var socket: Socket? = null

    override fun run() {
        while (running) {
            try {
                link()
            } catch (e: Exception) {
                if (running) Log.i(TAG, "r1d mic link: ${e.message}")
            }
            if (running) {
                try {
                    sleep(RETRY_MS)
                } catch (_: InterruptedException) {
                    return
                }
            }
        }
    }

    fun shutdown() {
        running = false
        socket?.close()
        interrupt()
    }

    private fun link() {
        Socket().use { sock ->
            socket = sock
            sock.tcpNoDelay = true
            sock.connect(InetSocketAddress("127.0.0.1", PORT), CONNECT_TIMEOUT_MS)
            val out = sock.getOutputStream()
            synchronized(out) {
                // r1d's per-boot token: only the host may be the mic (ADR-0026).
                val token = dev.r1os.host.net.R1d.token(reload = true).orEmpty()
                out.write("r1os-mic 1 $RATE $token\n".toByteArray())
                out.flush()
            }
            Log.i(TAG, "lent to r1d")
            val input = sock.getInputStream()
            var rec: Recorder? = null
            try {
                while (running) {
                    when (input.read()) {
                        -1 -> return
                        'S'.code -> if (rec == null) rec = Recorder(out).also { it.start() }
                        'E'.code -> {
                            rec?.finish()
                            rec = null
                        }
                    }
                }
            } finally {
                rec?.finish()
            }
        }
    }

    /** One recording: from r1d's `S` to its `E`. */
    private class Recorder(private val out: OutputStream) : Thread("r1os-mic-rec") {
        @Volatile private var recording = true

        @SuppressLint("MissingPermission") // MainActivity asks; without it AudioRecord won't initialise
        override fun run() {
            val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            val rec = try {
                AudioRecord(
                    MediaRecorder.AudioSource.VOICE_RECOGNITION,
                    RATE,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                    maxOf(min, CHUNK_SAMPLES * 2 * 4),
                )
            } catch (e: Exception) {
                Log.w(TAG, "can't open the mic: ${e.message}")
                return
            }
            if (rec.state != AudioRecord.STATE_INITIALIZED) {
                Log.w(TAG, "mic not initialised (is RECORD_AUDIO granted?)")
                rec.release()
                return
            }
            val samples = ShortArray(CHUNK_SAMPLES)
            val bytes = ByteArray(CHUNK_SAMPLES * 2)
            try {
                rec.startRecording()
                while (recording) {
                    val n = rec.read(samples, 0, samples.size)
                    if (n <= 0) continue
                    for (i in 0 until n) {
                        val s = samples[i].toInt()
                        bytes[2 * i] = (s and 0xff).toByte()
                        bytes[2 * i + 1] = (s shr 8 and 0xff).toByte()
                    }
                    synchronized(out) { out.write(bytes, 0, n * 2) }
                }
            } catch (e: Exception) {
                Log.i(TAG, "recording ended: ${e.message}")
            } finally {
                runCatching { rec.stop() }
                rec.release()
            }
        }

        fun finish() {
            recording = false
            join(JOIN_MS)
        }
    }

    private companion object {
        const val TAG = "r1os-mic"
        const val PORT = 7173
        const val RATE = 16000
        const val CHUNK_SAMPLES = 320 // 20 ms
        const val RETRY_MS = 1000L
        const val CONNECT_TIMEOUT_MS = 2000
        const val JOIN_MS = 500L
    }
}
