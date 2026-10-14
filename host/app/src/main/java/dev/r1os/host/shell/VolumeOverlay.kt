package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Region
import android.view.Choreographer
import android.view.View
import dev.r1os.host.ui.Spring
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.sin

/**
 * The volume (ADR-0014, ADR-0022): hold the button and turn the wheel. The screen fills with a solid
 * colour from the bottom like a liquid, up to the level: it springs as the wheel turns, its surface
 * waving harder the faster it moves, and settles flat. The level is a big number in the middle,
 * white over the dark and ink where the liquid covers it. Each step plays a note on the media
 * stream, so you hear the new volume.
 */
@SuppressLint("ViewConstructor")
class VolumeOverlay(context: Context) : View(context), Choreographer.FrameCallback {
    private var level = 0
    private var max = 15
    /** The liquid's height, 0 to 1: springy, so it sloshes a little. */
    private var fill = 0f
    private val spring = Spring { fill = it; invalidate() }
    private var phase = 0f
    private var lastFrame = 0L
    private var running = false

    private val liquid = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Theme.accent }
    private val numberLight = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Theme.FG
        typeface = Theme.font(700)
        textSize = context.dp(132f)
        textAlign = Paint.Align.CENTER
    }
    private val numberInk = Paint(numberLight).apply { color = Theme.ON_ACCENT }
    private val label = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Theme.MUTED
        typeface = Theme.font(600)
        textSize = context.dp(17f)
    }
    private val labelInk = Paint(label).apply { color = Theme.withAlpha(Theme.ON_ACCENT, 0.7f) }
    private val surface = Path()
    private val hide = Runnable {
        animate().alpha(0f).setDuration(220).withEndAction { visibility = GONE; stopWaves() }.start()
    }

    init {
        setBackgroundColor(Theme.BG)
        isClickable = true
        visibility = GONE
    }

    /** Volume mode began: the liquid starts where the volume is. */
    fun show(current: Int, max: Int) {
        removeCallbacks(hide)
        this.max = max.coerceAtLeast(1)
        level = current
        animate().cancel()
        if (visibility != VISIBLE) {
            visibility = VISIBLE
            alpha = 0f
            spring.snapTo(0f)
        }
        animate().alpha(1f).setDuration(120).start()
        spring.animateTo(level.toFloat() / this.max, STIFFNESS, DAMPING)
        startWaves()
    }

    /** A step: the liquid springs to the new level. */
    fun set(value: Int) {
        level = value
        spring.animateTo(level.toFloat() / max, STIFFNESS, DAMPING)
        startWaves()
        invalidate()
    }

    /** The button came up: it lingers a moment so you can read it, then fades. */
    fun hideSoon() {
        removeCallbacks(hide)
        postDelayed(hide, 650)
    }

    /** Gone at once (the display went off). */
    fun hideNow() {
        removeCallbacks(hide)
        animate().cancel()
        visibility = GONE
        stopWaves()
    }

    private fun startWaves() {
        if (running) return
        running = true
        lastFrame = 0L
        Choreographer.getInstance().postFrameCallback(this)
    }

    private fun stopWaves() {
        running = false
        Choreographer.getInstance().removeFrameCallback(this)
    }

    override fun doFrame(frameTimeNanos: Long) {
        if (!running) return
        val dt = if (lastFrame == 0L) 1f / 60 else ((frameTimeNanos - lastFrame) / 1e9f).coerceIn(0f, 0.05f)
        lastFrame = frameTimeNanos
        phase += dt * 5.5f
        invalidate()
        Choreographer.getInstance().postFrameCallback(this)
    }

    override fun onDraw(c: Canvas) {
        val w = width.toFloat()
        val h = height.toFloat()
        // The surface: a sine wave whose height follows how fast the level is moving, plus a calm ripple.
        val top = h * (1f - fill.coerceIn(0f, 1.04f))
        val amp = context.dp(3f) + context.dp(10f) * abs(spring.velocity).coerceAtMost(2f)
        surface.reset()
        surface.moveTo(0f, h)
        var x = 0f
        val step = w / 24
        surface.lineTo(0f, top + amp * sin(phase))
        while (x <= w) {
            surface.lineTo(x, top + amp * sin(phase + x / w * 2 * PI.toFloat() * 1.2f))
            x += step
        }
        surface.lineTo(w, h)
        surface.close()
        c.drawPath(surface, liquid)

        // The number, centred: white over the dark, ink inside the liquid.
        val text = level.toString()
        val cy = h * 0.5f - (numberLight.descent() + numberLight.ascent()) / 2
        c.save()
        c.clipPath(surface, Region.Op.DIFFERENCE)
        c.drawText(text, w / 2, cy, numberLight)
        c.restore()
        c.save()
        c.clipPath(surface)
        c.drawText(text, w / 2, cy, numberInk)
        c.restore()

        // "volume", top left, like a page title's caption.
        val lx = context.dp(20f)
        val ly = context.dp(36f)
        c.save()
        c.clipPath(surface, Region.Op.DIFFERENCE)
        c.drawText("volume", lx, ly, label)
        c.restore()
        c.save()
        c.clipPath(surface)
        c.drawText("volume", lx, ly, labelInk)
        c.restore()
    }

    private companion object {
        /** Springy enough to slosh, damped enough to settle quickly. */
        const val STIFFNESS = 210f
        const val DAMPING = 0.5f
    }
}
