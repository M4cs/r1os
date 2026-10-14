package dev.r1os.host.shell

import dev.r1os.host.Bluetooth
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import org.json.JSONObject

/**
 * Settings › Bluetooth (R7): on or off, your devices (connected ones first), and what's nearby to pair, all on the
 * wheel. A click on a paired device opens it (connect, disconnect, forget); on one nearby, it pairs. Scans for 12 s
 * when the page opens (and again from "look again"), never in the background.
 */
class BluetoothScreen(shell: Shell) : ListPage(shell, "Bluetooth") {
    private var off: (() -> Unit)? = null

    init {
        list.gap(6f)
        render()
    }

    override fun onShow() {
        off?.invoke()
        off = Bluetooth.onChange { render() }
        // The first time, the host's permissions come from r1d: the scan starts once they're there.
        Bluetooth.ensure { if (Bluetooth.on) Bluetooth.scan() }
        render()
    }

    override fun onHide() {
        super.onHide()
        off?.invoke()
        off = null
        Bluetooth.stop()
    }

    override fun onDestroy() = onHide()

    /** Built again whenever something's found or changes; the focus stays where it was. */
    private fun render() {
        val keep = list.focus
        build()
        if (keep >= 0 && list.size > 0) list.setFocus(keep.coerceAtMost(list.size - 1), animate = false)
    }

    private fun build(): Unit = list.rebuild {
        val on = Bluetooth.on
        item(if (on) "bluetooth" else "bluetooth-slash", "bluetooth", if (on) accent else null, sub = if (on && Bluetooth.name.isNotEmpty()) "others see it as ${Bluetooth.name}" else null, trailing = if (on) "on" else "off") {
            Sounds.play(Sounds.S.POP)
            if (!Bluetooth.setOn(context, !on)) shell.flash("couldn't switch it")
        }
        if (!on) return@rebuild
        if (!Bluetooth.allowed) {
            note("getting it ready…")
            return@rebuild
        }
        val all = Bluetooth.devices()
        val mine = all.filter { it.bonded }
        val near = all.filter { !it.bonded }
        if (mine.isNotEmpty()) section("your devices")
        for (d in mine) {
            item(icon(d.kind), d.name, if (d.connected) accent else null, sub = if (d.connected) "connected" else "not connected") {
                shell.push(BluetoothDevicePage(shell, d.address, d.name))
            }
        }
        section("nearby")
        for (d in near) {
            item(icon(d.kind), d.name, sub = if (d.bonding) "pairing…" else "click to pair") {
                Sounds.play(Sounds.S.POP)
                if (!Bluetooth.pair(d.address)) shell.flash("couldn't pair")
            }
        }
        if (Bluetooth.scanning) {
            note(if (near.isEmpty()) "looking for devices… (put yours in pairing mode)" else "still looking…")
        } else {
            item("arrows-clockwise", "look again", sub = if (near.isEmpty()) "nothing found: put yours in pairing mode" else null) { Bluetooth.scan() }
        }
    }
}

/** One paired device: connect or disconnect, and forget it. Without image v0.2.14, these open Android's settings. */
class BluetoothDevicePage(shell: Shell, private val address: String, name: String) : ListPage(shell, name) {
    private val confirm: Confirm = Confirm { render() }
    private var off: (() -> Unit)? = null

    init {
        list.gap(6f)
        render()
    }

    override fun onShow() {
        off?.invoke()
        off = Bluetooth.onChange { render() }
        render()
    }

    override fun onHide() {
        super.onHide()
        off?.invoke()
        off = null
    }

    override fun onDestroy() = onHide()

    /** Android's Bluetooth settings, for what the host can't do itself yet. */
    private fun android() {
        shell.flash("in android's bluetooth settings")
        R1d.post("/api/android/open", JSONObject().put("screen", "bluetooth"))
    }

    private fun render(): Unit = list.rebuild {
        val d = Bluetooth.devices().firstOrNull { it.address == address }
        if (d == null || !d.bonded) {
            note("it isn't paired any more")
            return@rebuild
        }
        note(if (d.connected) "connected" else "not connected")
        item(if (d.connected) "link-break" else "link", if (d.connected) "disconnect" else "connect", accent) {
            Sounds.play(Sounds.S.POP)
            if (!Bluetooth.connect(address, !d.connected)) android()
        }
        val armed = confirm.armed == "forget"
        item("trash", if (armed) "Click again to forget it" else "forget it", danger = armed, sub = "pair it again to use it") {
            confirm.tap("forget") {
                if (Bluetooth.forget(address)) shell.back() else android()
            }
        }
    }
}

/** A device's kind, as a Phosphor icon. */
private fun icon(kind: String) = when (kind) {
    "headphones" -> "headphones"
    "speaker" -> "speaker-hifi"
    "keyboard" -> "keyboard"
    "mouse" -> "mouse"
    "phone" -> "device-mobile"
    "computer" -> "laptop"
    "watch" -> "watch"
    "car" -> "car"
    else -> "bluetooth"
}
