package dev.r1os.host.ui

import android.view.Choreographer
import kotlin.math.abs
import kotlin.math.min
import kotlin.math.sqrt

/**
 * A damped spring on one value, stepped each frame. Retargeting keeps its velocity, so a run of
 * wheel ticks blends into one continuous motion instead of restarting an animation per tick (that
 * restart, with an overshoot curve, is what made the glance screen's unlock jitter back and forth).
 *
 * No AndroidX (ADR-0017): this is the little of `SpringAnimation` the shell needs.
 */
class Spring(private val onUpdate: (Float) -> Unit) : Choreographer.FrameCallback {
    var value = 0f
        private set
    var target = 0f
        private set
    /** Units per second: what the glance screen's agent stretches with. */
    var velocity = 0f
        private set
    private var stiffness = 240f
    private var damping = 2 * sqrt(240f)
    private var running = false
    private var lastFrame = 0L
    private var onRest: (() -> Unit)? = null

    /**
     * Springs toward `to`. `dampingRatio` 1 settles without overshoot; below 1 it gives a little.
     * `onRest` runs once it has settled there (not if it's retargeted first).
     */
    fun animateTo(to: Float, stiffness: Float, dampingRatio: Float, onRest: (() -> Unit)? = null) {
        target = to
        this.stiffness = stiffness
        damping = 2 * dampingRatio * sqrt(stiffness)
        this.onRest = onRest
        if (!running) {
            running = true
            lastFrame = 0L
            Choreographer.getInstance().postFrameCallback(this)
        }
    }

    /** Jumps there, still. */
    fun snapTo(v: Float) {
        cancel()
        value = v
        target = v
        velocity = 0f
        onUpdate(v)
    }

    fun cancel() {
        if (running) Choreographer.getInstance().removeFrameCallback(this)
        running = false
        onRest = null
        velocity = 0f
    }

    override fun doFrame(frameTimeNanos: Long) {
        if (!running) return
        val dt = if (lastFrame == 0L) 1f / 60 else ((frameTimeNanos - lastFrame) / 1e9f).coerceIn(0f, 1f / 20)
        lastFrame = frameTimeNanos
        // Semi-implicit Euler in small steps: stable at these stiffnesses whatever the frame rate.
        var left = dt
        while (left > 0f) {
            val h = min(STEP, left)
            velocity += (-stiffness * (value - target) - damping * velocity) * h
            value += velocity * h
            left -= h
        }
        if (abs(value - target) < REST_DELTA && abs(velocity) < REST_VELOCITY) {
            value = target
            velocity = 0f
            running = false
            onUpdate(value)
            val done = onRest
            onRest = null
            done?.invoke()
            return
        }
        onUpdate(value)
        Choreographer.getInstance().postFrameCallback(this)
    }

    private companion object {
        const val STEP = 1f / 240
        const val REST_DELTA = 0.0015f
        const val REST_VELOCITY = 0.02f
    }
}
