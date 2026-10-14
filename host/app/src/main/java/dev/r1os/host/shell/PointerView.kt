package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PointF
import android.view.View
import android.view.animation.LinearInterpolator
import android.widget.FrameLayout
import dev.r1os.host.AgentPointer
import dev.r1os.host.ui.Dim
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import kotlin.math.hypot

/**
 * An agent's pointer over a preview or a full screen (T-137): an arrow in the agent's colour, outlined so it shows on
 * light and dark pages, gliding to where the agent acts and pressing as it taps. Only transform and opacity animate,
 * and only while it moves; it fades after a few idle seconds; while it's hidden or the display's dimmed it's not
 * watched, so the agent doesn't wait for it (architecture §7).
 *
 * [map] turns the surface's own pixels (the page's, the app's display) into this view's, false while it can't yet
 * (not laid out). [tint] is the agent's colour.
 */
@SuppressLint("ViewConstructor")
class PointerView(
    context: Context,
    private val pointer: AgentPointer,
    small: Boolean,
    private val tint: () -> Int,
    private val map: (Float, Float, PointF) -> Boolean,
) : FrameLayout(context), AgentPointer.Mark {
    private val size = context.dp(if (small) 12f else 24f)
    private val arrow = Arrow(context)
    private val ripple = Ripple(context)
    private val pt = PointF()
    private var shown = false

    init {
        isClickable = false
        isFocusable = false
        clipChildren = false
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        arrow.alpha = 0f
        ripple.alpha = 0f
        addView(ripple, LayoutParams((size * 2.2f).toInt(), (size * 2.2f).toInt()))
        addView(arrow, LayoutParams(size.toInt(), size.toInt()))
        // The tip is the arrow's top-left corner; it presses from there.
        arrow.pivotX = 0f
        arrow.pivotY = 0f
    }

    override val watched: Boolean get() = shown && !Dim.dimmed && width > 0

    override fun glide(x: Float, y: Float, fromX: Float, fromY: Float): Long {
        if (!map(x, y, pt)) return 0
        arrow.color = tint()
        ripple.color = arrow.color
        val tx = pt.x
        val ty = pt.y
        arrow.animate().cancel()
        // Coming back after a rest: from where it last pointed (or in from below-right), then the glide.
        if (arrow.alpha < 0.05f) {
            if (!fromX.isNaN() && map(fromX, fromY, pt)) {
                arrow.translationX = pt.x
                arrow.translationY = pt.y
            } else {
                arrow.translationX = tx + size * 1.5f
                arrow.translationY = ty + size * 2f
            }
            arrow.scaleX = 1f
            arrow.scaleY = 1f
        }
        // A new act mid-glide re-aims from wherever it is.
        val dist = hypot(tx - arrow.translationX, ty - arrow.translationY) / resources.displayMetrics.density
        val ms = (GLIDE_MIN_MS + dist * 1.1f).toLong().coerceIn(GLIDE_MIN_MS, AgentPointer.MAX_GLIDE_MS)
        arrow.animate().translationX(tx).translationY(ty).alpha(1f).scaleX(1f).scaleY(1f)
            .setDuration(ms).setInterpolator(Theme.EASE_IN_OUT).start()
        rest()
        return ms
    }

    override fun press() {
        arrow.animate().cancel()
        arrow.animate().scaleX(0.78f).scaleY(0.78f).setDuration(80).setInterpolator(Theme.EASE_OUT)
            .withEndAction { arrow.animate().scaleX(1f).scaleY(1f).setDuration(140).setInterpolator(Theme.EASE_OUT).start() }.start()
        ripple.animate().cancel()
        ripple.translationX = arrow.translationX - ripple.width / 2f
        ripple.translationY = arrow.translationY - ripple.height / 2f
        ripple.scaleX = 0.3f
        ripple.scaleY = 0.3f
        ripple.alpha = 0.7f
        ripple.animate().scaleX(1f).scaleY(1f).alpha(0f).setDuration(380).setInterpolator(Theme.EASE_OUT).start()
        rest()
    }

    override fun drag(x: Float, y: Float, ms: Long) {
        if (!map(x, y, pt)) return
        arrow.animate().cancel()
        arrow.animate().translationX(pt.x).translationY(pt.y).alpha(1f).setDuration(ms).setInterpolator(LinearInterpolator()).start()
        rest()
    }

    override fun pulse() {
        if (arrow.alpha < 0.05f) return
        arrow.animate().cancel()
        arrow.animate().scaleX(1.25f).scaleY(1.25f).setDuration(110).setInterpolator(Theme.EASE_OUT)
            .withEndAction { arrow.animate().scaleX(1f).scaleY(1f).setDuration(160).setInterpolator(Theme.EASE_OUT).start() }.start()
        rest()
    }

    /** It fades once the agent's still for a while, and comes back where it left off. */
    private fun rest() {
        removeCallbacks(fade)
        postDelayed(fade, IDLE_MS)
    }

    private val fade = Runnable { arrow.animate().alpha(0f).setDuration(400).setInterpolator(Theme.EASE_IN_OUT).start() }

    override fun onVisibilityAggregated(isVisible: Boolean) {
        super.onVisibilityAggregated(isVisible)
        if (isVisible == shown) return
        shown = isVisible
        if (isVisible) {
            pointer.add(this)
        } else {
            pointer.remove(this)
            removeCallbacks(fade)
            arrow.animate().cancel()
            ripple.animate().cancel()
            arrow.alpha = 0f
            ripple.alpha = 0f
        }
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        pointer.remove(this)
        removeCallbacks(fade)
        shown = false
    }

    /** The arrow: drawn once (it moves by its transform), in the agent's colour with an outline. */
    private class Arrow(context: Context) : View(context) {
        private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
        private val edge = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeJoin = Paint.Join.ROUND
            color = Theme.FG
        }
        private val shade = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL; color = Theme.withAlpha(Theme.BG, 0.45f) }
        private val path = Path()
        var color = Theme.accent
            set(v) { if (field != v) { field = v; invalidate() } }

        override fun onSizeChanged(w: Int, h: Int, ow: Int, oh: Int) {
            // A classic arrow in a 20-unit box, its tip at the corner.
            val k = w / 20f
            path.reset()
            path.moveTo(1.5f * k, 1.5f * k)
            path.lineTo(1.5f * k, 16.5f * k)
            path.lineTo(5.6f * k, 12.9f * k)
            path.lineTo(8.4f * k, 18.8f * k)
            path.lineTo(11.0f * k, 17.6f * k)
            path.lineTo(8.3f * k, 11.8f * k)
            path.lineTo(13.6f * k, 11.8f * k)
            path.close()
            edge.strokeWidth = maxOf(1f, 1.4f * k)
        }

        override fun onDraw(c: Canvas) {
            c.save()
            c.translate(width * 0.06f, width * 0.06f)
            c.drawPath(path, shade)
            c.restore()
            fill.color = color
            c.drawPath(path, fill)
            c.drawPath(path, edge)
        }
    }

    /** The ring a tap leaves. */
    private class Ripple(context: Context) : View(context) {
        private val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
        var color = Theme.accent
            set(v) { if (field != v) { field = v; invalidate() } }

        override fun onDraw(c: Canvas) {
            p.color = color
            p.strokeWidth = width * 0.08f
            c.drawCircle(width / 2f, height / 2f, width / 2f - p.strokeWidth, p)
        }
    }

    private companion object {
        const val GLIDE_MIN_MS = 200L
        const val IDLE_MS = 3_000L
    }
}
