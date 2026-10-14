package dev.r1os.host

import android.app.Activity
import android.content.Intent
import android.os.Bundle

/**
 * Android's "allow USB debugging?" and "always allow on this network?" requests (roadmap R4's keep-list: r1os
 * asks in its own shell, so SystemUI can go). r1os's image points the framework at this activity
 * (`config_customAdbPublicKeyConfirmationComponent` and its Wi-Fi twin); only the system can start it (it needs
 * MANAGE_DEBUGGING). It hands the request to the shell and finishes; the shell answers through [Adb].
 */
class AdbConfirmActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val i = intent
        val ask = Intent(this, MainActivity::class.java).apply {
            action = ACTION
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        }
        when {
            i.getStringExtra("key") != null && i.getStringExtra("fingerprints") != null ->
                ask.putExtra("key", i.getStringExtra("key")).putExtra("fingerprints", i.getStringExtra("fingerprints"))
            i.getStringExtra("bssid") != null ->
                ask.putExtra("ssid", i.getStringExtra("ssid")).putExtra("bssid", i.getStringExtra("bssid"))
            else -> {
                finish()
                return
            }
        }
        startActivity(ask)
        finish()
    }

    companion object {
        const val ACTION = "dev.r1os.host.ADB_CONFIRM"
    }
}
