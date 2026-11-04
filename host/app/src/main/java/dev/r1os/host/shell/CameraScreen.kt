package dev.r1os.host.shell

import android.graphics.Color
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.LayerDrawable
import android.hardware.camera2.CaptureRequest
import android.media.MediaActionSound
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import dev.r1os.host.camera.Camera
import dev.r1os.host.camera.CameraPrefs
import dev.r1os.host.camera.Film
import dev.r1os.host.camera.Motor
import dev.r1os.host.camera.Photos
import dev.r1os.host.camera.Viewfinder
import dev.r1os.host.net.Ev
import dev.r1os.host.ui.Sounds
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import kotlin.math.roundToInt

/**
 * The camera (ADR-0031, ADR-0046), as an instrument after teenage engineering's. The preview fills the screen, the whole
 * 3:4 frame. A strip lies over its top: a lamp, *r1 cam*, a frame counter and **set**. A deck lies over its bottom:
 * four coloured knobs (film, exposure, white balance, zoom) and the **flip** key. Both are dark bands you can see
 * through.
 *
 * - **The wheel turns the lit knob**: film at first, or the knob you touched last. Touching a lit knob steps it on.
 * - **Flip** turns the module all the way round: to face you (seated against its stop, so it can't drift) or the
 *   world. The wheel doesn't turn it (Max, 2026-10-10: "just have flip").
 * - **A click** takes a photo, through the film you see (after the timer, if one is set; a click stops the timer).
 *   From a chat (`onPhoto`), it goes back to the chat for your next words; from Tools › camera it's kept in Photos,
 *   and its thumbnail shows in the corner.
 * - **Hold to talk** asks about what's in view: the photo is taken as you let go and goes with your words to your
 *   agent (`onPhoto` with the words, or a chat with the agent you last talked to).
 * - **Touching the picture** hides the bands, and brings them back. **Set** opens the camera's settings.
 * - **A double click** goes back; the camera turns back to where it was, and the dials go back to neutral for
 *   anything else that uses it (they're kept for next time).
 */
class CameraScreen(
    shell: Shell,
    private val source: String = "camera",
    /** A chat's camera: the photo (and the words, if you talked) go back to it. */
    private val onPhoto: ((id: String, words: String?) -> Unit)? = null,
) : Screen(shell) {
    private enum class Dial(val key: String) { FILM("film"), EXP("exp"), WB("wb"), ZOOM("zoom") }

    private val finder = Viewfinder(context)
    private val marks = FinderMarks(context)
    private val status = context.text(14f, 650, Color.WHITE).apply {
        gravity = Gravity.CENTER
        setPadding(context.dpi(12f), context.dpi(6f), context.dpi(12f), context.dpi(6f))
        background = roundRect(Color.argb(160, 0, 0, 0), context.dp(14f))
        visibility = View.GONE
    }
    private val last = ImageView(context).apply {
        scaleType = ImageView.ScaleType.CENTER_CROP
        background = roundRect(Color.argb(120, 0, 0, 0), context.dp(8f))
        clipToOutline = true
        visibility = View.GONE
        setOnClickListener { shell.open(Target.of("tool", "id" to "photos")) }
    }
    private val lamp = Led(context)
    private val counter = context.text(12f, 700, Color.BLACK).apply {
        typeface = Cam.MONO
        setPadding(context.dpi(6f), context.dpi(1f), context.dpi(6f), context.dpi(1f))
        background = roundRect(Cam.INK, context.dp(4f))
        setOnClickListener { shell.open(Target.of("tool", "id" to "photos")) }
    }
    private val setKey = CamKey(context, "set")
    private val flip = CamKey(context, "flip", CamKey.Glyph.FLIP).apply { color = Cam.TURN }
    private val knobs = mapOf(
        Dial.FILM to Knob(context, "film", Cam.FILM),
        Dial.EXP to Knob(context, "exposure", Cam.EXP),
        Dial.WB to Knob(context, "white", Cam.WB),
        Dial.ZOOM to Knob(context, "zoom", Cam.ZOOM),
    )
    /** The top band: a fade from dark, so the picture shows through under it. */
    private val strip = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(context.dpi(12f), context.dpi(4f), context.dpi(10f), context.dpi(14f))
        background = GradientDrawable(GradientDrawable.Orientation.TOP_BOTTOM,
            intArrayOf(Color.argb(190, 0, 0, 0), Color.argb(110, 0, 0, 0), Color.TRANSPARENT))
        addView(lamp, LinearLayout.LayoutParams(context.dpi(18f), context.dpi(18f)))
        addView(context.text(13f, 700, Cam.INK).apply { typeface = Cam.MONO; text = "r1 cam" },
            LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply { leftMargin = context.dpi(4f) })
        addView(counter)
        addView(setKey, LinearLayout.LayoutParams(context.dpi(46f), context.dpi(24f)).apply { leftMargin = context.dpi(8f) })
    }
    /** The bottom band: the knobs and flip, on dark glass under a hairline. */
    private val deck = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(context.dpi(4f), context.dpi(8f), context.dpi(10f), context.dpi(8f))
        background = LayerDrawable(arrayOf(
            ColorDrawable(Color.argb(170, 8, 9, 11)),
            GradientDrawable().apply { setColor(Color.TRANSPARENT); setStroke(context.dpi(1f), Color.argb(55, 255, 255, 255)) },
        )).apply { setLayerInset(1, -context.dpi(2f), 0, -context.dpi(2f), -context.dpi(2f)) }
        for (k in knobs.values) addView(k, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f))
        addView(flip, LinearLayout.LayoutParams(context.dpi(58f), context.dpi(66f)).apply { leftMargin = context.dpi(4f) })
    }
    override val view = FrameLayout(context).apply {
        setBackgroundColor(Color.BLACK)
        addView(finder, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        addView(marks, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        addView(strip, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, context.dpi(STRIP_DP), Gravity.TOP))
        addView(deck, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, context.dpi(DECK_DP), Gravity.BOTTOM))
        addView(status, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.TOP or Gravity.CENTER_HORIZONTAL).apply {
            topMargin = context.dpi(STRIP_DP)
        })
        addView(last, FrameLayout.LayoutParams(context.dpi(52f), context.dpi(52f), Gravity.BOTTOM or Gravity.START).apply {
            setMargins(context.dpi(10f), 0, 0, context.dpi(DECK_DP + 10f))
        })
    }

    private var dial = Dial.entries.firstOrNull { it.key == CameraPrefs.wheel } ?: Dial.FILM
    private var busy = false
    private var ready = false
    /** The bands are hidden: only the picture. */
    private var bare = false
    /** The timer's seconds left (0: not counting). */
    private var countdown = 0
    private val shutter = MediaActionSound().apply { load(MediaActionSound.SHUTTER_CLICK) }
    private val tick = Runnable { countDown() }

    init {
        CameraPrefs.init(context)
        finder.onError = { say(it, error = true) }
        finder.onReady = { ready = true; dials(); lampNow() }
        finder.setOnClickListener { setBare(!bare) }
        // The bands keep their touches: one between the keys isn't on the picture.
        strip.setOnClickListener {}
        deck.setOnClickListener {}
        setKey.setOnClickListener { shell.push(CameraSettingsScreen(shell)) }
        flip.setOnClickListener { flip() }
        for ((d, k) in knobs) k.setOnClickListener { if (dial == d) turn(1) else select(d) }
        Motor.onChange = { motorMoved() }
        Motor.turn(Motor.WORLD)
        motorMoved()
        setBare(false)
        settings()
        select(dial, sound = false)
    }

    // ---- the dials ------------------------------------------------------------------------------------

    private fun select(d: Dial, sound: Boolean = true) {
        dial = d
        CameraPrefs.wheel = d.key
        for ((k, v) in knobs) v.on = k == d
        if (sound) Sounds.play(Sounds.S.POP)
    }

    /** The lit dial, `steps` on (down the wheel, or clockwise: more). */
    private fun turn(steps: Int) {
        val moved = when (dial) {
            Dial.FILM -> {
                val all = Film.entries
                val i = (all.indexOf(CameraPrefs.film) + steps).coerceIn(0, all.size - 1)
                (all[i] != CameraPrefs.film).also { CameraPrefs.film = all[i] }
            }
            Dial.EXP -> {
                val (lo, hi) = evLimits()
                val k = (CameraPrefs.ev + steps).coerceIn(lo, hi)
                (k != CameraPrefs.ev).also { CameraPrefs.ev = k }
            }
            Dial.WB -> {
                val modes = wbModes()
                val i = (modes.indexOf(CameraPrefs.awb).coerceAtLeast(0) + steps).coerceIn(0, modes.size - 1)
                (modes[i] != CameraPrefs.awb).also { CameraPrefs.awb = modes[i] }
            }
            Dial.ZOOM -> {
                val z = (CameraPrefs.zoom + steps).coerceIn(0, zooms().size - 1)
                (z != CameraPrefs.zoom).also { CameraPrefs.zoom = z }
            }
        }
        Sounds.play(if (moved) Sounds.S.TICK else Sounds.S.BUMP)
        dials()
    }

    /** The dials onto the camera, the viewfinder and the knobs. */
    private fun dials() {
        val film = CameraPrefs.film
        val evs = evLimits()
        val ev = CameraPrefs.ev.coerceIn(evs.first, evs.second)
        // The camera's own exposure compensation if it has some; else a gain on the picture.
        val gain = if (hasEv()) 0f else ev / 3f
        finder.develop(film, gain)
        val modes = wbModes()
        val awb = CameraPrefs.awb.takeIf { it in modes } ?: CaptureRequest.CONTROL_AWB_MODE_AUTO
        val zs = zooms()
        val z = zs[CameraPrefs.zoom.coerceIn(0, zs.size - 1)]
        Camera.setControls(Camera.Controls(ev = if (hasEv()) evUnits(ev) else 0, awb = awb, zoom = z, gain = gain))
        knobs.getValue(Dial.FILM).apply { value = film.word; at = Film.entries.indexOf(film) / (Film.entries.size - 1f) }
        knobs.getValue(Dial.EXP).apply {
            value = if (ev == 0) "±0" else "%+.1f".format(ev / 3f)
            at = (ev - evs.first) / maxOf(1f, (evs.second - evs.first).toFloat())
        }
        knobs.getValue(Dial.WB).apply { value = WB_WORDS[awb] ?: "auto"; at = modes.indexOf(awb) / maxOf(1f, modes.size - 1f) }
        knobs.getValue(Dial.ZOOM).apply {
            value = (if (z % 1f == 0f) "%.1f".format(z) else z.toString()) + "×"
            at = zs.indexOf(z) / maxOf(1f, zs.size - 1f)
        }
    }

    /** The camera has exposure compensation of its own (Rabbit's reports none: 0..0). */
    private fun hasEv(): Boolean = Camera.caps?.let { it.ev.upper > it.ev.lower && it.evStep.toFloat() > 0f } == true

    /**
     * Exposure, in thirds of a stop: as far as the camera's own goes (±2 at most), or ±1 as a gain on the picture
     * (more only clips the highlights).
     */
    private fun evLimits(): Pair<Int, Int> {
        val c = Camera.caps ?: return -3 to 3
        if (!hasEv()) return -3 to 3
        val step = c.evStep.toFloat()
        val lo = maxOf(-6, kotlin.math.ceil(c.ev.lower * step * 3f - 0.01f).toInt())
        val hi = minOf(6, kotlin.math.floor(c.ev.upper * step * 3f + 0.01f).toInt())
        return lo to hi
    }

    private fun evUnits(thirds: Int): Int {
        val step = Camera.caps?.evStep?.toFloat()?.takeIf { it > 0f } ?: return 0
        return (thirds / 3f / step).roundToInt()
    }

    private fun wbModes(): List<Int> {
        val have = Camera.caps?.awb?.toSet()
        return WB_ORDER.filter { have == null || it in have }.ifEmpty { listOf(CaptureRequest.CONTROL_AWB_MODE_AUTO) }
    }

    private fun zooms(): List<Float> {
        val max = Camera.caps?.maxZoom ?: 4f
        return CameraPrefs.ZOOMS.filter { it <= max + 0.01f }.ifEmpty { listOf(1f) }
    }

    /** The settings page's: the grid, and the dials (reset there). */
    private fun settings() {
        marks.grid = CameraPrefs.grid
        Camera.mirrorSelfies = CameraPrefs.mirror
        finder.fit()
        counter.text = "%04d".format(CameraPrefs.frames)
        dials()
    }

    // ---- the motor ------------------------------------------------------------------------------------

    private fun flip() {
        Motor.face(user = !Motor.facingUser())
        Sounds.play(Sounds.S.POP)
    }

    private fun motorMoved() {
        flip.lit = Motor.moving
        finder.fit()
        lampNow()
    }

    private fun lampNow() {
        lamp.color = when {
            countdown > 0 -> Deck.REC
            Motor.moving -> Cam.TURN
            ready -> Cam.WB
            else -> Deck.DIM
        }
        lamp.glow = if (countdown > 0 || Motor.moving) 0.6f else 0f
    }

    // ---- shooting -------------------------------------------------------------------------------------

    /** The bands hidden (only the picture), or back. */
    private fun setBare(on: Boolean) {
        bare = on
        val v = if (on) View.GONE else View.VISIBLE
        strip.visibility = v
        deck.visibility = v
        if (on) last.visibility = View.GONE
        marks.top = if (on) 0f else context.dp(STRIP_DP)
        marks.bottom = if (on) 0f else context.dp(DECK_DP)
    }

    private fun say(msg: String, error: Boolean = false) {
        status.text = msg
        status.setTextColor(if (error) Theme.BAD else Color.WHITE)
        status.visibility = View.VISIBLE
        status.removeCallbacks(hideStatus)
        status.postDelayed(hideStatus, 2200)
    }

    private val hideStatus = Runnable { status.visibility = View.GONE }

    /** A click: the timer first, if one is set; a click while it counts stops it. */
    private fun click() {
        if (countdown > 0) {
            stopCountdown()
            say("stopped")
            return
        }
        val t = CameraPrefs.timer
        if (t <= 0) { shoot(null); return }
        countdown = t
        marks.count = t
        Sounds.play(Sounds.S.TICK)
        lampNow()
        view.postDelayed(tick, 1000)
    }

    private fun countDown() {
        countdown -= 1
        marks.count = countdown
        if (countdown <= 0) {
            lampNow()
            shoot(null)
            return
        }
        Sounds.play(Sounds.S.TICK)
        lamp.glow = if (countdown % 2 == 0) 0.6f else 0.1f
        view.postDelayed(tick, 1000)
    }

    private fun stopCountdown() {
        view.removeCallbacks(tick)
        countdown = 0
        marks.count = 0
        lampNow()
    }

    /** Takes the photo, through the film; `words`: you held to talk about it. */
    private fun shoot(words: String?) {
        if (busy) return
        busy = true
        // Facing you, the still is checked against what the preview shows (the HAL doesn't always turn it round).
        val stream = if (Camera.facingUser) finder.frame(STREAM_W, STREAM_H) else null
        finder.flash()
        if (CameraPrefs.shutter) shutter.play(MediaActionSound.SHUTTER_CLICK)
        lamp.color = Deck.REC
        Photos.take(context, source, CameraPrefs.film, stream = stream, onDone = { id ->
            busy = false
            lampNow()
            CameraPrefs.frames = CameraPrefs.frames + 1
            counter.text = "%04d".format(CameraPrefs.frames)
            val back = onPhoto
            when {
                back != null -> {
                    back(id, words)
                    shell.back()
                }
                words != null -> shell.talkAbout(id, words)
                else -> {
                    last.visibility = View.VISIBLE
                    Photos.show(last, id, context.dpi(52f))
                    say("saved to photos")
                }
            }
        }, onError = { busy = false; lampNow(); say(it, error = true) })
    }

    override fun onEvent(ev: Ev): Boolean {
        when {
            ev.type == "wheel" -> { turn(if (ev.int("delta") > 0) 1 else -1); return true }
            ev.isTap -> { click(); return true }
            ev.type == "voice" && ev.str("state") == "final" -> {
                val t = ev.str("text")?.trim().orEmpty()
                if (t.isEmpty()) return false
                stopCountdown()
                shoot(t)
                return true
            }
        }
        return false
    }

    override fun onShow() {
        settings()
        finder.resume()
    }

    override fun onHide() {
        stopCountdown()
        finder.pause()
    }

    override fun onDestroy() {
        stopCountdown()
        finder.pause()
        Motor.onChange = null
        Motor.putBack()
        Camera.setControls(Camera.Controls())
        shutter.release()
    }

    private companion object {
        /** The bands' heights. */
        const val STRIP_DP = 46f
        const val DECK_DP = 96f
        /** The preview's frame a selfie is checked against: small, 3:4 like the stream turned upright. */
        const val STREAM_W = 36
        const val STREAM_H = 48
        val WB_ORDER = listOf(
            CaptureRequest.CONTROL_AWB_MODE_AUTO, CaptureRequest.CONTROL_AWB_MODE_DAYLIGHT, CaptureRequest.CONTROL_AWB_MODE_CLOUDY_DAYLIGHT,
            CaptureRequest.CONTROL_AWB_MODE_SHADE, CaptureRequest.CONTROL_AWB_MODE_TWILIGHT, CaptureRequest.CONTROL_AWB_MODE_INCANDESCENT,
            CaptureRequest.CONTROL_AWB_MODE_FLUORESCENT, CaptureRequest.CONTROL_AWB_MODE_WARM_FLUORESCENT,
        )
        val WB_WORDS = mapOf(
            CaptureRequest.CONTROL_AWB_MODE_AUTO to "auto", CaptureRequest.CONTROL_AWB_MODE_DAYLIGHT to "sun",
            CaptureRequest.CONTROL_AWB_MODE_CLOUDY_DAYLIGHT to "cloud", CaptureRequest.CONTROL_AWB_MODE_SHADE to "shade",
            CaptureRequest.CONTROL_AWB_MODE_TWILIGHT to "dusk", CaptureRequest.CONTROL_AWB_MODE_INCANDESCENT to "bulb",
            CaptureRequest.CONTROL_AWB_MODE_FLUORESCENT to "tube", CaptureRequest.CONTROL_AWB_MODE_WARM_FLUORESCENT to "warm",
        )
    }
}

/**
 * The camera's settings (its **set** key): the timer, the grid, the shutter's sound; turning it to face out or you, and
 * re-homing it when it's lost where it is (ADR-0046); and the dials back to neutral.
 */
class CameraSettingsScreen(shell: Shell) : ListPage(shell, "camera") {
    init {
        render()
    }

    private fun render(): Unit = list.rebuild {
        val t = CameraPrefs.timer
        item("timer", "timer", if (t > 0) accent else null, sub = "a click starts it, another stops it", trailing = if (t == 0) "off" else "$t s") {
            CameraPrefs.timer = TIMERS[(TIMERS.indexOf(t) + 1) % TIMERS.size]
            render()
        }
        val grid = CameraPrefs.grid
        item("grid-nine", "grid", if (grid) accent else null, sub = "thirds over the viewfinder", trailing = if (grid) "on" else "off") {
            CameraPrefs.grid = !grid
            render()
        }
        val mirror = CameraPrefs.mirror
        item("arrows-left-right", "mirror selfies", if (mirror) accent else null, sub = "facing you: the preview and the photo", trailing = if (mirror) "on" else "off") {
            CameraPrefs.mirror = !mirror
            render()
        }
        val sound = CameraPrefs.shutter
        item(if (sound) "speaker-high" else "speaker-slash", "shutter sound", if (sound) accent else null, trailing = if (sound) "on" else "off") {
            CameraPrefs.shutter = !sound
            render()
        }
        section("turning")
        item("mountains", "face out", sub = "toward the world") { Motor.face(user = false); shell.back() }
        item("user-focus", "face you", sub = "for a selfie: it settles against its stop") { Motor.face(user = true); shell.back() }
        item("crosshair", "re-home it", sub = "lost? it finds its stop and comes back") {
            Motor.home()
            shell.back()
            shell.flash("re-homing")
        }
        section("the dials")
        item("arrow-counter-clockwise", "reset the dials", sub = "std, ±0, auto white, 1×") {
            CameraPrefs.resetDials()
            shell.flash("reset")
            render()
        }
        note("A photo is kept as you see it, through the film. Agents' own looks (\"what am I looking at?\") are plain.", size = 13f)
    }

    private companion object {
        val TIMERS = listOf(0, 3, 10)
    }
}
