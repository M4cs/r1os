package dev.r1os.host

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothClass
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.util.Log
import dev.r1os.host.net.R1d

/**
 * Bluetooth (R7): headphones, speakers and keyboards, on the wheel (Settings › Bluetooth). Android's own Bluetooth
 * stack, through its public API where that's enough (the list, scanning, pairing: BLUETOOTH_CONNECT and _SCAN, which
 * r1d grants the host), and its system API where it isn't: turning it on and off, connecting, disconnecting and
 * forgetting a device take BLUETOOTH_PRIVILEGED, which image v0.2.14 grants the host. Before that, turning it on or off
 * asks in Android's own dialog, and the rest opens Android's Bluetooth settings.
 *
 * Nothing scans unless Settings › Bluetooth is on screen, and then for 12 s at a time (battery first).
 */
@SuppressLint("MissingPermission") // each call is behind [allowed]
object Bluetooth {
    private const val TAG = "r1os"
    private const val CONNECT = "android.permission.BLUETOOTH_CONNECT"
    private const val SCAN = "android.permission.BLUETOOTH_SCAN"
    private const val PRIVILEGED = "android.permission.BLUETOOTH_PRIVILEGED"
    /** BluetoothProfile.HID_HOST (keyboards and mice), a hidden constant. */
    private const val HID_HOST = 4

    /** A device as the screen shows it. `kind`: headphones, speaker, keyboard, mouse, phone, computer, watch, car, other. */
    data class Device(val address: String, val name: String, val kind: String, val bonded: Boolean, val connected: Boolean, val bonding: Boolean)

    private val main = Handler(Looper.getMainLooper())
    private var app: Context? = null
    private var adapter: BluetoothAdapter? = null
    private val proxies = HashMap<Int, BluetoothProfile>()
    /** Devices found by the last scan, by address. */
    private val found = LinkedHashMap<String, BluetoothDevice>()
    var scanning = false
        private set
    private val listeners = mutableListOf<() -> Unit>()

    /** [f] runs (on the main thread) when anything shown changes; the returned function stops it. */
    fun onChange(f: () -> Unit): () -> Unit {
        listeners += f
        return { listeners -= f }
    }

    private fun changed() = main.post { listeners.toList().forEach { it() } }

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            when (i.action) {
                // Found, or its name came after (many send it a moment later).
                BluetoothDevice.ACTION_FOUND, BluetoothDevice.ACTION_NAME_CHANGED -> {
                    val d = device(i) ?: return
                    if (d.bondState == BluetoothDevice.BOND_NONE && !d.name.isNullOrBlank()) found[d.address] = d
                }
                BluetoothAdapter.ACTION_DISCOVERY_STARTED -> scanning = true
                BluetoothAdapter.ACTION_DISCOVERY_FINISHED -> scanning = false
                BluetoothAdapter.ACTION_STATE_CHANGED -> {
                    DeviceSettings.reportSoon()
                    // Switched off, Android forgets what a scan found (names too): so does the list.
                    if (on) proxies() else found.clear()
                }
                BluetoothDevice.ACTION_BOND_STATE_CHANGED -> device(i)?.let { if (it.bondState == BluetoothDevice.BOND_BONDED) found.remove(it.address) }
            }
            changed()
        }
    }

    private fun device(i: Intent): BluetoothDevice? =
        if (android.os.Build.VERSION.SDK_INT >= 33) i.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
        else @Suppress("DEPRECATION") i.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE)

    fun init(context: Context) {
        if (app != null) return
        val ctx = context.applicationContext
        app = ctx
        adapter = ctx.getSystemService(BluetoothManager::class.java)?.adapter
        val f = IntentFilter().apply {
            addAction(BluetoothDevice.ACTION_FOUND)
            addAction(BluetoothDevice.ACTION_NAME_CHANGED)
            addAction(BluetoothAdapter.ACTION_DISCOVERY_STARTED)
            addAction(BluetoothAdapter.ACTION_DISCOVERY_FINISHED)
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
            addAction(BluetoothDevice.ACTION_BOND_STATE_CHANGED)
            addAction(BluetoothDevice.ACTION_ACL_CONNECTED)
            addAction(BluetoothDevice.ACTION_ACL_DISCONNECTED)
            addAction("android.bluetooth.a2dp.profile.action.CONNECTION_STATE_CHANGED")
            addAction("android.bluetooth.headset.profile.action.CONNECTION_STATE_CHANGED")
        }
        ctx.registerReceiver(receiver, f, Context.RECEIVER_EXPORTED)
        if (allowed) proxies()
    }

    private fun has(p: String) = app?.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    /** The host may list, scan and pair (r1d grants these). */
    val allowed: Boolean get() = has(CONNECT) && has(SCAN)

    /** The host may also connect, disconnect, forget and switch it (image v0.2.14). */
    val privileged: Boolean get() = has(PRIVILEGED)

    /** Asks r1d for the permissions if Android hasn't given them; [then] once they're there. */
    fun ensure(then: () -> Unit = {}) {
        if (allowed) {
            ready()
            then()
            return
        }
        R1d.post("/api/android/bluetooth-access") {
            ready()
            changed()
            then()
        }
    }

    /** Allowed: the profile proxies, and the R1's own name for others to see (the image's was "TrebleDroid vanilla"). */
    private fun ready() {
        proxies()
        runCatching { if (adapter?.name.orEmpty().startsWith("TrebleDroid")) adapter?.setName(NAME) }
    }

    /** Android's profile proxies, which say what's connected: audio, calls, keyboards. */
    private fun proxies() {
        val a = adapter ?: return
        val ctx = app ?: return
        if (!allowed) return
        for (p in listOf(BluetoothProfile.A2DP, BluetoothProfile.HEADSET, HID_HOST)) {
            if (proxies.containsKey(p)) continue
            runCatching {
                a.getProfileProxy(ctx, object : BluetoothProfile.ServiceListener {
                    override fun onServiceConnected(profile: Int, proxy: BluetoothProfile) {
                        proxies[profile] = proxy
                        changed()
                    }
                    override fun onServiceDisconnected(profile: Int) {
                        proxies.remove(profile)
                    }
                }, p)
            }
        }
    }

    val on: Boolean get() = adapter?.isEnabled == true
    val name: String get() = if (allowed) runCatching { adapter?.name }.getOrNull().orEmpty() else ""

    /** Paired devices first (connected ones on top), then what the last scan found. */
    fun devices(): List<Device> {
        val a = adapter ?: return emptyList()
        if (!allowed || !on) return emptyList()
        val bonded = runCatching { a.bondedDevices.orEmpty() }.getOrDefault(emptySet())
            .map { d -> Device(d.address, d.name ?: d.address, kind(d), bonded = true, connected = connected(d), bonding = false) }
            .sortedWith(compareByDescending<Device> { it.connected }.thenBy { it.name.lowercase() })
        // Its name once: Android can forget it at any moment (Bluetooth switching off).
        val near = found.values.filter { d -> bonded.none { it.address == d.address } }
            .mapNotNull { d -> d.name?.takeIf { it.isNotBlank() }?.let { n -> d to n } }
            .map { (d, n) -> Device(d.address, n, kind(d), bonded = false, connected = false, bonding = d.bondState == BluetoothDevice.BOND_BONDING) }
        return bonded + near
    }

    private fun connected(d: BluetoothDevice): Boolean =
        proxies.values.any { p -> runCatching { p.getConnectionState(d) == BluetoothProfile.STATE_CONNECTED }.getOrDefault(false) } ||
            runCatching { BluetoothDevice::class.java.getMethod("isConnected").invoke(d) as Boolean }.getOrDefault(false)

    private fun kind(d: BluetoothDevice): String {
        val c = runCatching { d.bluetoothClass }.getOrNull() ?: return "other"
        return when (c.majorDeviceClass) {
            BluetoothClass.Device.Major.AUDIO_VIDEO -> when (c.deviceClass) {
                BluetoothClass.Device.AUDIO_VIDEO_LOUDSPEAKER, BluetoothClass.Device.AUDIO_VIDEO_PORTABLE_AUDIO, BluetoothClass.Device.AUDIO_VIDEO_HIFI_AUDIO -> "speaker"
                BluetoothClass.Device.AUDIO_VIDEO_CAR_AUDIO -> "car"
                else -> "headphones"
            }
            BluetoothClass.Device.Major.PERIPHERAL -> if (c.deviceClass and 0x80 != 0 && c.deviceClass and 0x40 == 0) "mouse" else "keyboard"
            BluetoothClass.Device.Major.PHONE -> "phone"
            BluetoothClass.Device.Major.COMPUTER -> "computer"
            BluetoothClass.Device.Major.WEARABLE -> "watch"
            else -> "other"
        }
    }

    /**
     * Bluetooth on or off: at once with the privileged permission; else Android asks in its own dialog (the only way an
     * app may since Android 13). False when it couldn't even ask.
     */
    fun setOn(context: Context, want: Boolean): Boolean {
        val a = adapter ?: return false
        if (privileged) {
            val done = runCatching { @Suppress("DEPRECATION") if (want) a.enable() else a.disable() }.getOrDefault(false)
            if (done) return true
        }
        val action = if (want) BluetoothAdapter.ACTION_REQUEST_ENABLE else "android.bluetooth.adapter.action.REQUEST_DISABLE"
        return runCatching { context.startActivity(Intent(action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }.isSuccess
    }

    /** Looks for devices for 12 s (only while Settings › Bluetooth shows). */
    fun scan() {
        val a = adapter ?: return
        if (!allowed || !on) return
        found.clear()
        runCatching {
            if (a.isDiscovering) a.cancelDiscovery()
            a.startDiscovery()
            main.removeCallbacks(stopScan)
            main.postDelayed(stopScan, SCAN_MS)
        }.onFailure { Log.w(TAG, "bluetooth: can't scan", it) }
        changed()
    }

    private val stopScan = Runnable { stop() }

    fun stop() {
        main.removeCallbacks(stopScan)
        runCatching { if (adapter?.isDiscovering == true) adapter?.cancelDiscovery() }
        scanning = false
        changed()
    }

    /** Pairs with a device the scan found (Android shows its own code dialog if the device wants one). */
    fun pair(address: String): Boolean {
        val d = found[address] ?: return false
        runCatching { adapter?.cancelDiscovery() }
        return runCatching { d.createBond() }.getOrDefault(false).also { changed() }
    }

    private fun bonded(address: String): BluetoothDevice? = runCatching { adapter?.bondedDevices?.firstOrNull { it.address == address } }.getOrNull()

    /** Connects or disconnects a paired device (its system API). False without the permission: open Android's settings. */
    fun connect(address: String, want: Boolean): Boolean {
        if (!privileged) return false
        val d = bonded(address) ?: return false
        return runCatching { BluetoothDevice::class.java.getMethod(if (want) "connect" else "disconnect").invoke(d); true }
            .onFailure { Log.w(TAG, "bluetooth: couldn't ${if (want) "connect" else "disconnect"}", it) }
            .getOrDefault(false)
            .also { changed() }
    }

    /** Forgets a paired device (its system API). False without the permission: open Android's settings. */
    fun forget(address: String): Boolean {
        if (!privileged) return false
        val d = bonded(address) ?: return false
        return runCatching { BluetoothDevice::class.java.getMethod("removeBond").invoke(d) as Boolean }.getOrDefault(false).also { changed() }
    }

    private const val SCAN_MS = 12_000L
    /** What other devices see. */
    private const val NAME = "R1"
}
