package dev.r1os.host.shell

import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.Sounds
import org.json.JSONObject

/**
 * Settings › Controls (R7): the wheel's direction, and what a triple press of the button opens from anywhere (off by
 * default: with a shortcut set, a double click waits a moment to see whether a third press follows). r1d keeps both
 * (`invert_wheel`, `triple_press`) and turns the wheel round itself, for everything r1os does with it.
 */
class ControlsScreen(shell: Shell) : ListPage(shell, "Controls") {
    private var device: JSONObject? = null

    init {
        list.gap(6f)
        load()
    }

    private fun load(): Unit = R1d.get("/api/device") { r -> device = r.optJSONObject("device") ?: r; render() }

    override fun onShow() = load()

    override fun onAnyEvent(ev: Ev) {
        if (ev.type == "device_set") load()
    }

    private fun change(key: String, v: Any) {
        Sounds.play(Sounds.S.POP)
        R1d.put("/api/device/change", JSONObject().put("changes", JSONObject().put(key, v))) { r ->
            device = r.optJSONObject("device") ?: device
            render()
        }
    }

    private fun render(): Unit = list.rebuild {
        val d = device ?: run { note("…"); return@rebuild }
        val inverted = d.optBoolean("invert_wheel")
        item("arrows-down-up", "turn the wheel the other way", if (inverted) accent else null, sub = "up and down swap, everywhere in r1os", trailing = if (inverted) "on" else "off") {
            change("invert_wheel", !inverted)
        }
        section("a triple press opens")
        val now = d.optString("triple_press", "off")
        for ((key, words) in TRIPLE) {
            val on = key == now
            item(if (on) "check-circle" else "circle", words.first, if (on) accent else null, sub = words.second) { change("triple_press", key) }
        }
    }

    private companion object {
        /** r1d's choices, with their words. */
        val TRIPLE = listOf(
            "off" to ("nothing" to "a double click stays instant"),
            "recorder" to ("the recorder" to "ready to record"),
            "camera" to ("the camera" to "ready to shoot"),
            "journal" to ("the journal" to "today's page"),
            "timer" to ("timers" to "set one on the wheel"),
            "whats_up" to ("what's up" to "what's new, at a glance"),
        )
    }
}

/**
 * Settings › Power (R7): turning the R1 off, or restarting it, each with a second click. r1d asks Android's power service
 * (image v0.2.14 lets it); if it couldn't, its error shows here.
 */
class PowerScreen(shell: Shell) : ListPage(shell, "Power") {
    private val confirm: Confirm = Confirm { render() }
    private var said: String? = null

    init {
        list.gap(6f)
        render()
    }

    private fun power(action: String) {
        Sounds.play(Sounds.S.POP)
        R1d.post("/api/power", JSONObject().put("action", action), onError = { said = it; render() }) {}
    }

    private fun render(): Unit = list.rebuild {
        said?.let { note(it) }
        val off = confirm.armed == "off"
        item("power", if (off) "Click again to turn it off" else "turn off", danger = off, sub = "the side button turns it on again") {
            confirm.tap("off") { power("off") }
        }
        val restart = confirm.armed == "restart"
        item("arrow-clockwise", if (restart) "Click again to restart" else "restart", danger = restart, sub = "back in about a minute") {
            confirm.tap("restart") { power("restart") }
        }
    }
}
