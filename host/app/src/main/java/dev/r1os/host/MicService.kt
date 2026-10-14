package dev.r1os.host

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.util.Log

/**
 * Keeps push-to-talk working while another Android app is in front (ADR-0027).
 *
 * Android only lets an app record while it's in the foreground, or while it runs a foreground service of
 * type `microphone` that it started from the foreground. [Mic] does the recording; this service only
 * holds that state for the host's process. It records nothing itself, and it can't start without the
 * mic permission. Its notification has nowhere to show: the R1 has no status bar (ADR-0014).
 *
 * It's started each time the host comes to the front, so it comes back if it was stopped. It isn't
 * sticky: Android won't let a service of this type start again from the background.
 */
class MicService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Push-to-talk", NotificationManager.IMPORTANCE_MIN))
        val n = Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentTitle("Hold the button to talk")
            .setOngoing(true)
            .build()
        try {
            startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
        } catch (e: RuntimeException) {
            Log.w(TAG, "mic service: not in the foreground", e)
            stopSelf()
        }
        return START_NOT_STICKY
    }

    companion object {
        private const val TAG = "r1os"
        private const val CHANNEL = "mic"
        private const val ID = 1

        /** Starts it if the host may record (call while the host is in front). */
        fun start(context: Context) {
            if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) return
            runCatching { context.startForegroundService(Intent(context, MicService::class.java)) }
                .onFailure { Log.w(TAG, "mic service: start", it) }
        }

        fun stop(context: Context) {
            runCatching { context.stopService(Intent(context, MicService::class.java)) }
        }

        private var lock: android.os.PowerManager.WakeLock? = null

        /**
         * The Recorder is recording (ADR-0033): the CPU stays awake with the screen off, so the mic keeps
         * streaming to r1d. Held at most a few hours, like the recording.
         */
        fun recording(context: Context, on: Boolean) {
            if (on) {
                if (lock?.isHeld == true) return
                val pm = context.getSystemService(android.os.PowerManager::class.java)
                lock = pm.newWakeLock(android.os.PowerManager.PARTIAL_WAKE_LOCK, "r1os:recorder").apply { acquire(4 * 3600_000L) }
            } else {
                lock?.let { if (it.isHeld) it.release() }
                lock = null
            }
        }
    }
}
