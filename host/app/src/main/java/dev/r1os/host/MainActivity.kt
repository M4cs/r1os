package dev.r1os.host

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import android.view.KeyEvent
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.webkit.WebView
import dev.r1os.host.shell.Shell
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme

/**
 * The r1os host: the R1's home screen (ADR-0002, ADR-0017).
 *
 * The shell is native ([Shell]): home, push-to-talk and chats are Android views, and each mini-app
 * gets its own WebView only while it's open. The host does no input handling of its own: r1d reads
 * the wheel and side button straight from evdev and streams them to the shell, so the matching
 * Android key events are swallowed here.
 *
 * It also lends r1d the microphone for push-to-talk ([Mic]): Android only lets a foreground app
 * record. The host is the R1's foreground app, and [MicService] keeps the mic while another app is in
 * front (ADR-0027).
 */
class MainActivity : Activity() {
    private lateinit var shell: Shell
    private val mic = Mic()

    /**
     * The shell's own scale (ADR-0021): 213 dpi instead of the system's 180, so it lays out on a
     * 360x480 dp canvas and everything is drawn larger, like OS3's big, clear UI. Only this app;
     * Android's own screens keep the system density.
     */
    override fun attachBaseContext(newBase: Context) {
        super.attachBaseContext(newBase)
        applyOverrideConfiguration(Configuration().apply { densityDpi = UI_DENSITY })
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Theme.init(this)
        // r1os's keyboard turns this screen on its side while you type (ADR-0055).
        dev.r1os.host.ime.Rotation.activity = this
        // The look (ADR-0043), as last seen: the shell starts in it before r1d answers.
        dev.r1os.host.ui.Look.load(this)
        // The wardrobe (T-062) is read once, off the main thread, before the first character draws from it.
        Thread({ dev.r1os.host.ui.Wardrobe.catalog }, "r1os wardrobe").start()
        window.setBackgroundDrawable(android.graphics.drawable.ColorDrawable(Theme.BG))
        Sounds.init(this)
        dev.r1os.host.ui.Haptics.init(this)
        Bluetooth.init(this)
        DeviceSettings.init(this)
        // Android apps you've left alone close in the background (T-150).
        BackgroundApps.init(this)
        // chrome://inspect over adb for the mini-app pages. Fine on a personal device; revisit for R4.
        WebView.setWebContentsDebuggingEnabled(true)
        // The glance screen is the R1's lock screen (ADR-0019): show over Android's own keyguard,
        // which the shell dismisses when you go home.
        setShowWhenLocked(true)
        shell = Shell(this)
        setContentView(shell)
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), REQ_MIC)
        }
        mic.start()
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    private var started = false

    /** The look changed (ADR-0043): the shell is built again in it, back on the screens it was showing. */
    fun rebuildShell() {
        val saved = shell.saved()
        val glanceUp = shell.glanceUp
        shell.destroy()
        // Kept app pages were made in the old look (their mode and font are in their address).
        dev.r1os.host.shell.KeptPages.clear()
        window.setBackgroundDrawable(android.graphics.drawable.ColorDrawable(Theme.BG))
        shell = Shell(this)
        setContentView(shell)
        if (started) shell.setShown(true)
        shell.restore(saved, glanceUp)
    }

    /** Requests other parts of the system send the shell: Android's adb confirmations ([AdbConfirmActivity]). */
    private fun handle(i: Intent?) {
        if (i?.action != AdbConfirmActivity.ACTION) return
        shell.askAdb(i.getStringExtra("key"), i.getStringExtra("fingerprints"), i.getStringExtra("ssid"), i.getStringExtra("bssid"))
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQ_MIC) MicService.start(this)
    }

    override fun onStart() {
        super.onStart()
        started = true
        shell.setShown(true)
        // Push-to-talk keeps the mic once another app is in front (ADR-0027). Only startable from here.
        MicService.start(this)
    }

    override fun onResume() {
        super.onResume()
        hideSystemBars()
        AndroidBars.off(this)
        BackgroundApps.onHome()
    }

    override fun onStop() {
        started = false
        shell.setShown(false)
        super.onStop()
    }

    private fun hideSystemBars() {
        window.insetsController?.apply {
            hide(WindowInsets.Type.systemBars())
            systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        // Every key keeps the display from dimming, the swallowed ones too (T-036).
        dev.r1os.host.ui.Dim.poke()
        return when (event.keyCode) {
            // The wheel (Generic.kl maps its KEY_UP/KEY_DOWN to DPAD) and the side button
            // (keylayout: BUTTON_1): r1d already delivers these to the shell.
            KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_BUTTON_1,
            // Home is the root: there's nothing behind it to go back to.
            KeyEvent.KEYCODE_BACK -> true
            else -> super.dispatchKeyEvent(event)
        }
    }

    /** A touch (and the keys that reach `super`): the display won't dim for a while (T-036). */
    override fun onUserInteraction() {
        super.onUserInteraction()
        dev.r1os.host.ui.Dim.poke()
    }

    override fun onDestroy() {
        if (dev.r1os.host.ime.Rotation.activity === this) dev.r1os.host.ime.Rotation.activity = null
        MicService.stop(this)
        mic.shutdown()
        shell.destroy()
        super.onDestroy()
    }

    private companion object {
        const val REQ_MIC = 1
        const val UI_DENSITY = 213
    }
}
