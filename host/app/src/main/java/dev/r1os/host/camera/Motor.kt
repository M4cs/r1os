package dev.r1os.host.camera

import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import org.json.JSONObject

/**
 * The camera module's motor (r1d's `/api/motor`, 0–180°): turned toward the world, or around to you. r1d moves it one
 * move at a time, the latest target winning (ADR-0046), and says where it's going and where it came to rest (`motor
 * {deg, moving}`); here only the latest target is in flight, so a fast turn of the wheel doesn't queue up requests.
 *
 * Rabbit's camera HAL moves it too: to face out when the camera opens, to 90 when it closes. So once the camera has
 * opened, the target goes again ([cameraOpened]), and putting it back waits for the HAL's park ([putBack]).
 */
object Motor {
    /** Facing the world (away from the screen), and facing you. */
    const val WORLD = 180
    const val USER = 0
    /** Long enough for one of the HAL's moves (180° in ~0.47 s) to end. */
    const val HAL_SETTLE_MS = 700L

    /** Where it's going, or is. */
    var deg = 90
        private set
    var moving = false
        private set
    /** Told when [deg] or [moving] change (the camera's gauge). */
    var onChange: (() -> Unit)? = null
    private var sending = false
    private var pending: JSONObject? = null
    /** Where it was before the camera turned it, to put it back. */
    private var rest: Int? = null

    /** Turns to `d` (clamped). The camera's picture follows: facing you, it's upside down. */
    fun turn(d: Int) {
        remember()
        aim(d.coerceIn(0, 180))
        send(JSONObject().put("deg", deg))
    }

    /** Faces you, seated against its stop (so any drift is undone), or the world. */
    fun face(user: Boolean) {
        remember()
        aim(if (user) USER else WORLD)
        send(JSONObject().put("face", if (user) "you" else "world"))
    }

    /** It lost track (it was blocked or pushed): r1d runs it to its stop to find itself, then back to where it is. */
    fun home() {
        moving = true
        onChange?.invoke()
        send(JSONObject().put("home", true).put("deg", deg))
    }

    fun facingUser(d: Int = deg) = if (USER < WORLD) d < 90 else d > 90

    /** Back where it was before the camera turned it, once the HAL has parked it (the camera closing). */
    fun putBack() {
        val r = rest ?: return
        rest = null
        aim(r)
        Camera.facingUser = false
        R1d.main.postDelayed({ if (rest == null) send(JSONObject().put("deg", r)) }, HAL_SETTLE_MS)
    }

    /** The camera opened, and the HAL turned the module to face out: ours again (facing you, seated). */
    fun cameraOpened() {
        if (rest == null) return
        send(if (deg == USER) JSONObject().put("face", "you") else JSONObject().put("deg", deg))
    }

    /** r1d's `motor` events: where it's going, and where it came to rest. */
    fun onEvent(ev: Ev) {
        if (ev.type != "motor") return
        deg = ev.int("deg")
        moving = ev.bool("moving")
        Camera.facingUser = facingUser(deg)
        onChange?.invoke()
    }

    private fun remember() {
        if (rest == null) rest = R1d.state?.optJSONObject("motor")?.optInt("deg", 90) ?: 90
    }

    private fun aim(d: Int) {
        deg = d
        moving = true
        Camera.facingUser = facingUser(deg)
        onChange?.invoke()
    }

    private fun send(body: JSONObject) {
        pending = body
        if (sending) return
        val next = pending ?: return
        pending = null
        sending = true
        R1d.post("/api/motor", next, onError = { sending = false; pending?.let { send(it) } }) {
            sending = false
            pending?.let { send(it) }
        }
    }
}
